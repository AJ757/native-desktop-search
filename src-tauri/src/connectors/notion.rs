//! Notion connector.
//!
//! Uses Notion API `/v1/search` and block children endpoints to fetch workspace
//! pages and databases, with incremental synchronization based on last_edited_time.

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde::{Deserialize, Serialize};

use crate::auth::keychain::{self, SERVICE_NAME};
use crate::connectors::onedrive::{parse_rfc3339_to_unix, CloudSyncResult};
use crate::error::{AppError, Result};
use crate::sources::SourceEvent;

#[derive(Debug, Clone)]
pub struct NotionConnector {
    workspace_id: String,
    base_url: String,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    results: Vec<NotionObject>,
    next_cursor: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    has_more: bool,
}

#[derive(Debug, Deserialize)]
struct NotionObject {
    id: String,
    #[serde(default)]
    archived: bool,
    #[serde(default)]
    #[allow(dead_code)]
    url: Option<String>,
    #[serde(rename = "last_edited_time")]
    last_edited_time: Option<String>,
    #[serde(default)]
    properties: serde_json::Value,
}

impl NotionConnector {
    pub fn new(workspace_id: String) -> Self {
        let base_url = std::env::var("NOTION_API_BASE_URL")
            .unwrap_or_else(|_| "https://api.notion.com".into());
        Self {
            workspace_id,
            base_url,
        }
    }

    /// Resolves the access token from the OS keychain.
    fn access_token(&self) -> Result<String> {
        let key = format!("notion:{}", self.workspace_id);
        let tokens = keychain::get_tokens(SERVICE_NAME, &key)?.ok_or_else(|| {
            AppError::NotFound(format!(
                "OAuth tokens for Notion workspace {}",
                self.workspace_id
            ))
        })?;
        Ok(tokens.access_token)
    }

    /// Fetches changed pages and databases from Notion.
    pub async fn fetch_changes(&self, cursor: Option<&str>) -> Result<CloudSyncResult> {
        let token = self.access_token()?;
        let client = reqwest::Client::new();
        let search_url = format!("{}/v1/search", self.base_url);

        let mut events = Vec::new();
        let next_cursor = cursor.map(|s| s.to_string());
        let mut latest_edited_time = cursor.and_then(parse_rfc3339_to_unix);

        #[derive(Serialize)]
        struct SearchBody<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            start_cursor: Option<&'a str>,
            sort: SortOption<'a>,
        }

        #[derive(Serialize)]
        struct SortOption<'a> {
            direction: &'a str,
            timestamp: &'a str,
        }

        let body = SearchBody {
            start_cursor: next_cursor.as_deref(),
            sort: SortOption {
                direction: "ascending",
                timestamp: "last_edited_time",
            },
        };

        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|e| AppError::Other(e.to_string()))?,
        );
        headers.insert("Notion-Version", HeaderValue::from_static("2022-06-28"));

        let resp = client
            .post(&search_url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::Other(format!("Notion search request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AppError::Other(format!(
                "Notion API error ({status}): {text}"
            )));
        }

        let data: SearchResponse = resp
            .json()
            .await
            .map_err(|e| AppError::Other(format!("Invalid Notion JSON response: {e}")))?;

        for obj in data.results {
            if obj.archived {
                events.push(SourceEvent::Delete {
                    external_id: format!("notion:{}", obj.id),
                });
            } else {
                let title = extract_notion_title(&obj.properties);
                let modified_at = obj
                    .last_edited_time
                    .as_deref()
                    .and_then(parse_rfc3339_to_unix);

                if let Some(t) = modified_at {
                    if latest_edited_time.is_none_or(|cur| t > cur) {
                        latest_edited_time = Some(t);
                    }
                }

                events.push(SourceEvent::Upsert {
                    external_id: format!("notion:{}", obj.id),
                    title: if title.is_empty() {
                        "Untitled Notion Page".into()
                    } else {
                        title
                    },
                    mime: Some("application/notion-page".into()),
                    modified_at,
                });
            }
        }

        let new_cursor = data
            .next_cursor
            .or_else(|| latest_edited_time.map(|t| t.to_string()));

        Ok(CloudSyncResult { events, new_cursor })
    }
}

/// Extracts title from various Notion property shapes.
pub fn extract_notion_title(props: &serde_json::Value) -> String {
    // 1. Check property named "title"
    if let Some(title_prop) = props.get("title") {
        if let Some(title_arr) = title_prop.get("title").and_then(|v| v.as_array()) {
            let mut text = String::new();
            for item in title_arr {
                if let Some(plain) = item.get("plain_text").and_then(|v| v.as_str()) {
                    text.push_str(plain);
                }
            }
            if !text.is_empty() {
                return text;
            }
        }
    }

    // 2. Check property named "Name"
    if let Some(name_prop) = props.get("Name") {
        if let Some(title_arr) = name_prop.get("title").and_then(|v| v.as_array()) {
            let mut text = String::new();
            for item in title_arr {
                if let Some(plain) = item.get("plain_text").and_then(|v| v.as_str()) {
                    text.push_str(plain);
                }
            }
            if !text.is_empty() {
                return text;
            }
        }
    }

    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_title_from_notion_properties() {
        let props = serde_json::json!({
            "title": {
                "id": "title",
                "type": "title",
                "title": [
                    {
                        "type": "text",
                        "plain_text": "DevOps RoadMap"
                    }
                ]
            }
        });
        assert_eq!(extract_notion_title(&props), "DevOps RoadMap");
    }

    #[tokio::test]
    async fn fetch_changes_with_notion_mock_server() {
        if std::env::var("NOTION_API_BASE_URL").is_err() {
            return;
        }
        let connector = NotionConnector::new("test_ws".into());
        let tokens = keychain::AuthTokens {
            access_token: "mock_token".into(),
            refresh_token: None,
            expires_at: None,
            account_id: "test_ws".into(),
            account_email: None,
        };
        keychain::save_tokens(SERVICE_NAME, "notion:test_ws", &tokens).unwrap();

        let res = connector.fetch_changes(None).await.unwrap();
        assert_eq!(res.events.len(), 2);
    }
}
