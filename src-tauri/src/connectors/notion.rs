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

//add new imports

use std::time::Duration;

const NOTION_VERSION: &str = "2022-06-28";
/// How many levels of nested blocks (toggles, columns, lists) to read.
const MAX_BLOCK_DEPTH: usize = 4;
/// Stop reading a page once this much text has been collected.
const MAX_TEXT_BYTES: usize = 1_000_000;
/// Block lists fetched at once. Notion allows about three requests per
/// second on average and answers bursts beyond that with HTTP 429.
const PARALLEL_REQUESTS: usize = 3;
/// Tries per request before giving up.
const MAX_ATTEMPTS: u32 = 4;

#[derive(Debug, Deserialize)]
struct BlockList {
    results: Vec<serde_json::Value>,
    next_cursor: Option<String>,
    #[serde(default)]
    has_more: bool,
}

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

    /// Fetches pages and databases edited since the last sync.
    ///
    /// `cursor` is the newest edit time (Unix seconds) seen by the previous
    /// sync; it is ours, never sent to Notion. Results come newest first, so
    /// paging stops at the first item older than the cursor. Without a usable
    /// cursor (first sync, or a value saved by an older version) everything
    /// is listed.
    pub async fn fetch_changes(&self, cursor: Option<&str>) -> Result<CloudSyncResult> {
        let token = self.access_token()?;
        let client = reqwest::Client::new();
        let search_url = format!("{}/v1/search", self.base_url);
        let headers = Self::auth_headers(&token)?;

        let since = cursor.and_then(|c| c.parse::<i64>().ok());
        let mut latest_edited_time = since;
        let mut events = Vec::new();
        let mut page_cursor: Option<String> = None;

        #[derive(Serialize)]
        struct SearchBody<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            start_cursor: Option<&'a str>,
            page_size: u32,
            sort: SortOption<'a>,
        }

        #[derive(Serialize)]
        struct SortOption<'a> {
            direction: &'a str,
            timestamp: &'a str,
        }

        'pages: loop {
            let body = SearchBody {
                start_cursor: page_cursor.as_deref(),
                page_size: 100,
                sort: SortOption {
                    direction: "descending",
                    timestamp: "last_edited_time",
                },
            };

            let (status, text) = send_with_retry(
                || {
                    client
                        .post(&search_url)
                        .headers(headers.clone())
                        .json(&body)
                },
                "Notion search request",
            )
            .await?;
            if !status.is_success() {
                return Err(AppError::Other(format!(
                    "Notion API error ({status}): {text}"
                )));
            }
            let data: SearchResponse = serde_json::from_str(&text)
                .map_err(|e| AppError::Other(format!("Invalid Notion JSON response: {e}")))?;

            for obj in data.results {
                let modified_at = obj
                    .last_edited_time
                    .as_deref()
                    .and_then(parse_rfc3339_to_unix);

                // Notion rounds edit times to the minute, so items edited in
                // the same minute as the cursor are listed again; re-listing
                // an unchanged page does not re-index it.
                if let (Some(t), Some(since)) = (modified_at, since) {
                    if t < since {
                        break 'pages;
                    }
                }
                if let Some(t) = modified_at {
                    if latest_edited_time.is_none_or(|cur| t > cur) {
                        latest_edited_time = Some(t);
                    }
                }

                if obj.archived {
                    events.push(SourceEvent::Delete {
                        external_id: format!("notion:{}", obj.id),
                    });
                } else {
                    let title = extract_notion_title(&obj.properties);
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

            match (data.has_more, data.next_cursor) {
                (true, Some(next)) => page_cursor = Some(next),
                _ => break,
            }
        }

        Ok(CloudSyncResult {
            events,
            new_cursor: latest_edited_time.map(|t| t.to_string()),
        })
    }

    fn auth_headers(token: &str) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|e| AppError::Other(e.to_string()))?,
        );
        headers.insert("Notion-Version", HeaderValue::from_static(NOTION_VERSION));
        Ok(headers)
    }

    /// Lists the direct child blocks of a page or block, following pagination.
    async fn list_children(
        &self,
        client: &reqwest::Client,
        token: &str,
        block_id: &str,
    ) -> Result<Vec<serde_json::Value>> {
        let url = format!("{}/v1/blocks/{block_id}/children", self.base_url);
        let headers = Self::auth_headers(token)?;
        let mut blocks = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let mut query: Vec<(&str, String)> = vec![("page_size", "100".into())];
            if let Some(c) = &cursor {
                query.push(("start_cursor", c.clone()));
            }

            let (status, body) = send_with_retry(
                || client.get(&url).headers(headers.clone()).query(&query),
                "Notion block request",
            )
            .await?;

            // Databases and some block types have no readable children.
            if status.as_u16() == 400 {
                return Ok(blocks);
            }
            if !status.is_success() {
                return Err(AppError::Other(format!(
                    "Notion block API error ({status}): {body}"
                )));
            }

            let page: BlockList = serde_json::from_str(&body)
                .map_err(|e| AppError::Other(format!("Invalid Notion block JSON: {e}")))?;
            blocks.extend(page.results);

            match (page.has_more, page.next_cursor) {
                (true, Some(next)) => cursor = Some(next),
                _ => break,
            }
        }
        Ok(blocks)
    }

    /// Collects a page's text (paragraphs, headings, lists, to-dos, quotes,
    /// table cells and so on) for indexing. `page_id` is the bare Notion ID,
    /// without the `notion:` prefix.
    ///
    /// Nested blocks are read level by level, a few requests at a time.
    pub async fn download_text(&self, page_id: &str) -> Result<String> {
        let token = self.access_token()?;
        let client = reqwest::Client::new();
        let mut text = String::new();
        let mut level = vec![page_id.to_string()];

        for depth in 0..=MAX_BLOCK_DEPTH {
            let mut next_level = Vec::new();

            for group in level.chunks(PARALLEL_REQUESTS) {
                let tasks: Vec<_> = group
                    .iter()
                    .map(|block_id| {
                        let (this, client, token, block_id) = (
                            self.clone(),
                            client.clone(),
                            token.clone(),
                            block_id.clone(),
                        );
                        tokio::spawn(
                            async move { this.list_children(&client, &token, &block_id).await },
                        )
                    })
                    .collect();

                for task in tasks {
                    let blocks = task.await.map_err(|e| {
                        AppError::Internal(format!("Notion download task failed: {e}"))
                    })??;

                    for block in blocks {
                        let line = block_text(&block);
                        if !line.is_empty() {
                            text.push_str(&line);
                            text.push('\n');
                        }

                        // Sub-pages are separate documents, so don't read into them here.
                        let kind = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        let has_children = block
                            .get("has_children")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        if has_children
                            && depth < MAX_BLOCK_DEPTH
                            && kind != "child_page"
                            && kind != "child_database"
                        {
                            if let Some(id) = block.get("id").and_then(|v| v.as_str()) {
                                next_level.push(id.to_string());
                            }
                        }
                    }
                    if text.len() >= MAX_TEXT_BYTES {
                        return Ok(text);
                    }
                }
            }

            if next_level.is_empty() {
                break;
            }
            level = next_level;
        }
        Ok(text)
    }
}

/// Sends a request, retrying rate limits (HTTP 429), server errors, dropped
/// connections and responses cut off mid-body, which Notion produces now and
/// then. Returns the status and body of the last attempt.
async fn send_with_retry(
    request: impl Fn() -> reqwest::RequestBuilder,
    what: &str,
) -> Result<(reqwest::StatusCode, String)> {
    for attempt in 1..=MAX_ATTEMPTS {
        let last = attempt == MAX_ATTEMPTS;
        let backoff = Duration::from_secs(1u64 << attempt); // 2, 4, 8 s

        let resp = match request().send().await {
            Ok(resp) => resp,
            Err(e) if !last => {
                tracing::debug!("{what} failed (attempt {attempt}): {e}; retrying");
                tokio::time::sleep(backoff).await;
                continue;
            }
            Err(e) => return Err(AppError::Other(format!("{what} failed: {e}"))),
        };

        let status = resp.status();
        if !last && (status.as_u16() == 429 || status.is_server_error()) {
            let wait = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .map(Duration::from_secs)
                .unwrap_or(backoff)
                .min(Duration::from_secs(30));
            tokio::time::sleep(wait).await;
            continue;
        }

        match resp.text().await {
            Ok(body) => return Ok((status, body)),
            Err(e) if !last => {
                tracing::debug!("{what}: response cut off (attempt {attempt}): {e}; retrying");
                tokio::time::sleep(backoff).await;
            }
            Err(e) => {
                return Err(AppError::Other(format!(
                    "{what}: could not read response: {e}"
                )))
            }
        }
    }
    unreachable!("the last attempt always returns")
}

//new functions

fn plain_text(items: &[serde_json::Value]) -> String {
    items
        .iter()
        .filter_map(|i| i.get("plain_text").and_then(|p| p.as_str()))
        .collect::<Vec<_>>()
        .join("")
}

/// Pulls the visible text out of one Notion block.
fn block_text(block: &serde_json::Value) -> String {
    let Some(kind) = block.get("type").and_then(|t| t.as_str()) else {
        return String::new();
    };
    let Some(body) = block.get(kind) else {
        return String::new();
    };
    if let Some(rich) = body.get("rich_text").and_then(|v| v.as_array()) {
        return plain_text(rich);
    }
    match kind {
        "child_page" | "child_database" => body
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string(),
        "table_row" => body
            .get("cells")
            .and_then(|c| c.as_array())
            .map(|cells| {
                cells
                    .iter()
                    .filter_map(|cell| cell.as_array().map(|c| plain_text(c)))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default(),
        _ => String::new(),
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

        // The saved cursor is an edit time, and the next sync must accept it.
        let cursor = res.new_cursor.expect("first sync saves a cursor");
        assert!(cursor.parse::<i64>().is_ok(), "cursor {cursor:?}");
        connector.fetch_changes(Some(&cursor)).await.unwrap();

        // Nothing in the mock workspace was edited after 2030.
        let later = connector.fetch_changes(Some("1900000000")).await.unwrap();
        assert!(later.events.is_empty());

        // Cursors saved by older versions (Notion page cursors) mean a full listing.
        let legacy = connector
            .fetch_changes(Some("2b85ecb1-9aac-81a7-a645-eab8825a36ed"))
            .await
            .unwrap();
        assert_eq!(legacy.events.len(), 2);
    }
}
