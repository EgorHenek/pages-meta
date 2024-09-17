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

    walk(dom.document, &mut page_info, false);

    Ok(page_info)
}

fn walk(handle: Handle, page_info: &mut PageInfo, mut is_head: bool) {
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
        walk(child.clone(), page_info, is_head);
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
}
