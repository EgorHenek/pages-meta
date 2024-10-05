use axum::{
    http::StatusCode,
    response::{IntoResponse, Response, Result},
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

#[derive(Debug, Serialize, Default, Deserialize)]
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
) -> Result<Response, ServerError> {
    // Validate URL
    url.validate()?;

    let decoded_url = percent_decode_str(&url.url)
        .decode_utf8()
        .unwrap()
        .to_string();

    match fetch_html(&decoded_url).await {
        Ok((status, html)) => {
            if status.is_success() {
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
                Ok(Json(page_info).into_response())
            } else {
                let body = Json(serde_json::json!({
                    "error": {
                        "code": status.as_u16(),
                        "message": status.canonical_reason().unwrap_or("Unknown error")
                    }
                }));
                Ok((status, body).into_response())
            }
        }
        Err(err) => {
            let status = err.status().unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            let body = Json(serde_json::json!({
                "error": {
                    "code": status.as_u16(),
                    "message": err.to_string()
                }
            }));
            Ok((status, body).into_response())
        }
    }
}

fn trim_url(url: &str) -> Result<String, url::ParseError> {
    let url = Url::parse(url)?;
    let host = url.host_str().unwrap_or("");
    let scheme = url.scheme();
    let port = url.port().map(|p| format!(":{}", p)).unwrap_or_default();
    Ok(format!("{}://{}{}", scheme, host, port))
}

async fn fetch_html(url: &str) -> Result<(StatusCode, String), reqwest::Error> {
    let response = reqwest::get(url).await?;
    let status = response.status();
    let text = response.text().await?;
    Ok((status, text))
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
mod tests {
    use super::*;
    use axum::{body, http::StatusCode};

    #[tokio::test]
    async fn test_handle_extract_success() {
        let mut server = mockito::Server::new_async().await;
        let url = server.url();

        let _m = server
            .mock("GET", "/")
            .with_status(200)
            .with_header("content-type", "text/html")
            .with_body(
                r#"
                <html>
                <head>
                    <title>Test Page</title>
                    <meta name="description" content="This is a test page">
                    <link rel="icon" href="/favicon.ico">
                    <meta property="og:title" content="OG Test Title">
                    <meta property="og:description" content="OG Test Description">
                </head>
                <body></body>
                </html>
            "#,
            )
            .create_async()
            .await;

        let url_path = UrlPath { url: url.clone() };
        let result = handle_extract(ValidatedPath(url_path)).await.unwrap();

        assert_eq!(result.status(), StatusCode::OK);

        let body = body::to_bytes(result.into_body(), usize::MAX)
            .await
            .unwrap();
        let page_info: PageInfo = serde_json::from_slice(&body).unwrap();

        assert_eq!(page_info.title, Some("Test Page".to_string()));
        assert_eq!(
            page_info.description,
            Some("This is a test page".to_string())
        );
        assert_eq!(page_info.favicon, Some("/favicon.ico".to_string()));

        let og_tags = page_info.og_tags.unwrap();
        assert_eq!(
            og_tags.get("title").unwrap().as_str().unwrap(),
            "OG Test Title"
        );
        assert_eq!(
            og_tags.get("description").unwrap().as_str().unwrap(),
            "OG Test Description"
        );
    }

    #[tokio::test]
    async fn test_handle_extract_not_found() {
        let mut server = mockito::Server::new_async().await;
        let url = server.url();

        let _m = server
            .mock("GET", "/")
            .with_status(404)
            .create_async()
            .await;

        let url_path = UrlPath { url: url.clone() };
        let result = handle_extract(ValidatedPath(url_path)).await.unwrap();

        assert_eq!(result.status(), StatusCode::NOT_FOUND);

        let body = body::to_bytes(result.into_body(), usize::MAX)
            .await
            .unwrap();
        let error: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(error["error"]["code"], 404);
        assert_eq!(error["error"]["message"], "Not Found");
    }

    #[tokio::test]
    async fn test_handle_extract_invalid_url() {
        let url_path = UrlPath {
            url: "not a valid url".to_string(),
        };
        let result = handle_extract(ValidatedPath(url_path)).await;

        assert!(result.is_err());

        if let Err(ServerError::ValidationError(err)) = result {
            assert!(err.field_errors().contains_key("url"));
        } else {
            panic!("Expected ValidationError");
        }
    }

    #[tokio::test]
    async fn test_handle_extract_with_manifest() {
        let mut server = mockito::Server::new_async().await;
        let url = server.url();

        let _m = server
            .mock("GET", "/")
            .with_status(200)
            .with_header("content-type", "text/html")
            .with_body(
                r#"
                <html>
                <head>
                    <title>Test Page</title>
                    <link rel="manifest" href="/manifest.json">
                </head>
                <body></body>
                </html>
            "#,
            )
            .create_async()
            .await;

        let _manifest_mock = server
            .mock("GET", "/manifest.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"
                {
                    "name": "Test App",
                    "short_name": "Test"
                }
            "#,
            )
            .create_async()
            .await;

        let url_path = UrlPath { url: url.clone() };
        let result = handle_extract(ValidatedPath(url_path)).await.unwrap();

        assert_eq!(result.status(), StatusCode::OK);

        let body = body::to_bytes(result.into_body(), usize::MAX)
            .await
            .unwrap();
        let page_info: PageInfo = serde_json::from_slice(&body).unwrap();

        assert_eq!(page_info.name, Some("Test App".to_string()));
        assert_eq!(page_info.short_name, Some("Test".to_string()));
        assert_eq!(page_info.manifest, Some(format!("{}/manifest.json", url)));
    }

    #[tokio::test]
    async fn test_handle_extract_multiple_og_images() {
        let mut server = mockito::Server::new_async().await;
        let url = server.url();

        let _m = server
            .mock("GET", "/")
            .with_status(200)
            .with_header("content-type", "text/html")
            .with_body(
                r#"
                <html>
                <head>
                    <title>Test Page</title>
                    <meta property="og:image" content="image1.jpg">
                    <meta property="og:image:width" content="800">
                    <meta property="og:image:height" content="600">
                    <meta property="og:image" content="image2.jpg">
                    <meta property="og:image:width" content="1200">
                    <meta property="og:image:height" content="900">
                </head>
                <body></body>
                </html>
            "#,
            )
            .create_async()
            .await;

        let url_path = UrlPath { url: url.clone() };
        let result = handle_extract(ValidatedPath(url_path)).await.unwrap();

        assert_eq!(result.status(), StatusCode::OK);

        let body = body::to_bytes(result.into_body(), usize::MAX)
            .await
            .unwrap();
        let page_info: PageInfo = serde_json::from_slice(&body).unwrap();

        let og_tags = page_info.og_tags.unwrap();
        let images = og_tags.get("image").unwrap().as_array().unwrap();
        assert_eq!(images.len(), 2);
        assert_eq!(
            images[0].get("url").unwrap().as_str().unwrap(),
            "image1.jpg"
        );
        assert_eq!(images[0].get("width").unwrap().as_str().unwrap(), "800");
        assert_eq!(images[0].get("height").unwrap().as_str().unwrap(), "600");
        assert_eq!(
            images[1].get("url").unwrap().as_str().unwrap(),
            "image2.jpg"
        );
        assert_eq!(images[1].get("width").unwrap().as_str().unwrap(), "1200");
        assert_eq!(images[1].get("height").unwrap().as_str().unwrap(), "900");
    }
}
