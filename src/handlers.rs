use axum::{
    http::StatusCode,
    response::{IntoResponse, Result},
    Json,
};
use html5ever::parse_document;
use html5ever::tendril::TendrilSink;
use markup5ever_rcdom::{Handle, NodeData, RcDom};
use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use url::Url;
use validator::{Validate, ValidationError};

use crate::{errors::ServerError, extractors::ValidatedPath};

#[derive(Debug, Deserialize, Validate)]
pub struct UrlPath {
    #[validate(
        url(message = "Invalid URL"),
        custom(
            function = "validate_schema",
            message = "URL must start with http:// or https://"
        )
    )]
    url: String,
}

#[derive(Debug, Serialize, Default)]
pub struct PageInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    favicon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    short_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    manifest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    og_tags: Option<HashMap<String, serde_json::Value>>,
}

fn validate_schema(url: &str) -> Result<(), ValidationError> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(ValidationError::new("bad_schema"));
    }
    Ok(())
}

pub async fn handle_health() -> impl IntoResponse {
    StatusCode::OK
}
pub async fn handle_extract(
    ValidatedPath(url): ValidatedPath<UrlPath>,
) -> Result<Json<PageInfo>, ServerError> {
    let decoded_url = percent_decode_str(&url.url)
        .decode_utf8()
        .unwrap()
        .to_string();

    let html = fetch_html(&decoded_url).await?;
    let mut page_info = extract_info(&html).await?;
    if let Some(manifest) = &mut page_info.manifest {
        let base_url = trim_url(&decoded_url)?;
        *manifest = format!("{}{}", base_url, manifest);

        let json = fetch_json(manifest).await?;

        page_info.short_name = json
            .get("short_name")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string());
        page_info.name = json
            .get("name")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string());
    }
    Ok(Json(page_info))
}

fn trim_url(url: &str) -> Result<String, url::ParseError> {
    let url = Url::parse(url)?;
    let host = url.host_str().unwrap_or("");
    let scheme = url.scheme();
    Ok(format!("{}://{}", scheme, host))
}

async fn fetch_html(url: &str) -> Result<String, ServerError> {
    let body = reqwest::get(url).await?.text().await?;

    Ok(body)
}

async fn fetch_json(url: &str) -> Result<serde_json::Value, ServerError> {
    let body: serde_json::Value = reqwest::get(url).await?.json().await?;

    Ok(body)
}

async fn extract_info(html: &str) -> Result<PageInfo, ServerError> {
    let dom = parse_document(RcDom::default(), Default::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())?;

    let mut page_info = PageInfo::default();
    let mut og_tags = HashMap::new();

    walk(dom.document, &mut page_info, &mut og_tags, false);

    if !og_tags.is_empty() {
        page_info.og_tags = Some(og_tags);
    }

    Ok(page_info)
}

fn walk(
    handle: Handle,
    page_info: &mut PageInfo,
    og_tags: &mut HashMap<String, serde_json::Value>,
    mut is_head: bool,
) {
    let node = handle;
    if let NodeData::Element {
        ref name,
        ref attrs,
        ..
    } = node.data
    {
        let tag_name = name.local.as_ref();
        if tag_name == "title" && is_head {
            if let Some(first_child) = node.children.borrow().first() {
                if let NodeData::Text { ref contents } = first_child.data {
                    page_info.title = Some(contents.borrow().to_string());
                }
            }
        } else if tag_name == "head" {
            is_head = true;
        } else if tag_name == "meta" {
            let attrs = attrs.borrow();
            if attrs.iter().any(|attr| {
                attr.name.local.as_ref() == "name" && attr.value.as_ref() == "description"
            }) {
                if let Some(content) = attrs
                    .iter()
                    .find(|attr| attr.name.local.as_ref() == "content")
                {
                    page_info.description = Some(content.value.to_string());
                }
            } else if attrs.iter().any(|attr| {
                attr.name.local.as_ref() == "property" && attr.value.as_ref().starts_with("og:")
            }) {
                if let Some(property) = attrs
                    .iter()
                    .find(|attr| attr.name.local.as_ref() == "property")
                {
                    if let Some(content) = attrs
                        .iter()
                        .find(|attr| attr.name.local.as_ref() == "content")
                    {
                        let full_key = property.value.as_ref();
                        let key = full_key.trim_start_matches("og:");
                        let value = content.value.to_string();

                        if key.starts_with("image")
                            || key.starts_with("audio")
                            || key.starts_with("video")
                        {
                            let main_key = if key.starts_with("image") {
                                "image"
                            } else if key.starts_with("audio") {
                                "audio"
                            } else {
                                "video"
                            };
                            let entry = og_tags
                                .entry(main_key.to_string())
                                .or_insert_with(|| serde_json::Value::Array(Vec::new()));

                            if let Some(array) = entry.as_array_mut() {
                                if key == main_key {
                                    array.push(serde_json::json!({ "url": value }));
                                } else {
                                    let attr = key.split_once(':').map(|x| x.1).unwrap_or(key);
                                    if let Some(last_obj) = array.last_mut() {
                                        if let Some(obj) = last_obj.as_object_mut() {
                                            obj.insert(
                                                attr.to_string(),
                                                serde_json::Value::String(value),
                                            );
                                        }
                                    }
                                }
                            }
                        } else if key == "locale:alternate" {
                            let entry = og_tags
                                .entry("locale:alternate".to_string())
                                .or_insert_with(|| serde_json::Value::Array(Vec::new()));

                            if let Some(array) = entry.as_array_mut() {
                                array.push(serde_json::Value::String(value));
                            }
                        } else {
                            og_tags.insert(key.to_string(), serde_json::Value::String(value));
                        }
                    }
                }
            }
        } else if tag_name == "link" {
            let attrs = attrs.borrow();
            if attrs
                .iter()
                .any(|attr| attr.name.local.as_ref() == "rel" && attr.value.as_ref() == "icon")
            {
                if let Some(content) = attrs.iter().find(|attr| attr.name.local.as_ref() == "href")
                {
                    page_info.favicon = Some(content.value.to_string());
                }
            }
            if attrs
                .iter()
                .any(|attr| attr.name.local.as_ref() == "rel" && attr.value.as_ref() == "manifest")
            {
                if let Some(content) = attrs.iter().find(|attr| attr.name.local.as_ref() == "href")
                {
                    page_info.manifest = Some(content.value.to_string());
                }
            }
        }
    }

    for child in node.children.borrow().iter() {
        walk(child.clone(), page_info, og_tags, is_head);
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_validate_schema() {
        let url = "https://google.com";

        let result = validate_schema(url);

        assert!(result.is_ok())
    }

    #[test]
    fn test_validate_schema_when_ftp() {
        let url = "ftp://gogle.com";

        let result = validate_schema(url);

        assert!(result.is_err())
    }

    #[tokio::test]
    async fn test_extract_info_when_many_titles() {
        let html = "<html><head><title>Head title</title></head><body><svg><title>Body title</title></svg></body></html>";

        let page_info = extract_info(html).await.unwrap();

        assert_eq!(page_info.title.unwrap(), "Head title")
    }

    #[tokio::test]
    async fn test_extract_og_tags() {
        let html = r#"
        <html>
        <head>
            <meta property="og:title" content="OG Title">
            <meta property="og:description" content="OG Description">
            <meta property="og:image" content="https://example.com/image1.jpg">
            <meta property="og:image:width" content="1200">
            <meta property="og:image:height" content="630">
            <meta property="og:image:alt" content="Example image 1">
            <meta property="og:image" content="https://example.com/image2.jpg">
            <meta property="og:audio" content="https://example.com/audio.mp3">
            <meta property="og:audio:type" content="audio/mpeg">
            <meta property="og:video" content="https://example.com/video.mp4">
            <meta property="og:video:width" content="1280">
            <meta property="og:video:height" content="720">
            <meta property="og:video:type" content="video/mp4">
            <meta property="og:locale:alternate" content="fr_FR">
            <meta property="og:locale:alternate" content="es_ES">
        </head>
        <body></body>
        </html>
        "#;

        let page_info = extract_info(html).await.unwrap();

        let og_tags = page_info.og_tags.unwrap();
        assert_eq!(og_tags.get("title").unwrap().as_str().unwrap(), "OG Title");
        assert_eq!(
            og_tags.get("description").unwrap().as_str().unwrap(),
            "OG Description"
        );

        let images = og_tags.get("image").unwrap().as_array().unwrap();
        assert_eq!(images.len(), 2);
        assert_eq!(
            images[0]
                .as_object()
                .unwrap()
                .get("url")
                .unwrap()
                .as_str()
                .unwrap(),
            "https://example.com/image1.jpg"
        );
        assert_eq!(
            images[0]
                .as_object()
                .unwrap()
                .get("width")
                .unwrap()
                .as_str()
                .unwrap(),
            "1200"
        );
        assert_eq!(
            images[0]
                .as_object()
                .unwrap()
                .get("height")
                .unwrap()
                .as_str()
                .unwrap(),
            "630"
        );
        assert_eq!(
            images[0]
                .as_object()
                .unwrap()
                .get("alt")
                .unwrap()
                .as_str()
                .unwrap(),
            "Example image 1"
        );
        assert_eq!(
            images[1]
                .as_object()
                .unwrap()
                .get("url")
                .unwrap()
                .as_str()
                .unwrap(),
            "https://example.com/image2.jpg"
        );

        let audios = og_tags.get("audio").unwrap().as_array().unwrap();
        assert_eq!(audios.len(), 1);
        assert_eq!(
            audios[0]
                .as_object()
                .unwrap()
                .get("url")
                .unwrap()
                .as_str()
                .unwrap(),
            "https://example.com/audio.mp3"
        );
        assert_eq!(
            audios[0]
                .as_object()
                .unwrap()
                .get("type")
                .unwrap()
                .as_str()
                .unwrap(),
            "audio/mpeg"
        );

        let videos = og_tags.get("video").unwrap().as_array().unwrap();
        assert_eq!(videos.len(), 1);
        assert_eq!(
            videos[0]
                .as_object()
                .unwrap()
                .get("url")
                .unwrap()
                .as_str()
                .unwrap(),
            "https://example.com/video.mp4"
        );
        assert_eq!(
            videos[0]
                .as_object()
                .unwrap()
                .get("width")
                .unwrap()
                .as_str()
                .unwrap(),
            "1280"
        );
        assert_eq!(
            videos[0]
                .as_object()
                .unwrap()
                .get("height")
                .unwrap()
                .as_str()
                .unwrap(),
            "720"
        );
        assert_eq!(
            videos[0]
                .as_object()
                .unwrap()
                .get("type")
                .unwrap()
                .as_str()
                .unwrap(),
            "video/mp4"
        );

        let locale_alternates = og_tags.get("locale:alternate").unwrap().as_array().unwrap();
        assert_eq!(locale_alternates.len(), 2);
        assert_eq!(locale_alternates[0].as_str().unwrap(), "fr_FR");
        assert_eq!(locale_alternates[1].as_str().unwrap(), "es_ES");
    }
}
