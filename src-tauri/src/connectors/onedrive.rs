//! Microsoft 365 and OneDrive connector.
//!
//! Uses Microsoft Graph Delta API (`/v1.0/me/drive/root/delta`) to fetch
//! drive items and track incremental changes via delta tokens.

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde::Deserialize;

use crate::auth::keychain::{self, SERVICE_NAME};
use crate::error::{AppError, Result};
use crate::sources::SourceEvent;

#[derive(Debug, Clone)]
pub struct OneDriveConnector {
    account_id: String,
    base_url: String,
}

#[derive(Debug, Clone)]
pub struct CloudSyncResult {
    pub events: Vec<SourceEvent>,
    pub new_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeltaResponse {
    #[serde(rename = "@odata.deltaLink")]
    delta_link: Option<String>,
    #[serde(rename = "@odata.nextLink")]
    next_link: Option<String>,
    #[serde(default)]
    value: Vec<DriveItem>,
}

#[derive(Debug, Deserialize)]
struct DriveItem {
    id: String,
    #[serde(default)]
    name: String,
    file: Option<FileInfo>,
    deleted: Option<DeletedInfo>,
    #[serde(rename = "lastModifiedDateTime")]
    last_modified_date_time: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FileInfo {
    #[serde(rename = "mimeType")]
    mime_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeletedInfo {
    #[serde(default)]
    #[allow(dead_code)]
    state: Option<String>,
}

impl OneDriveConnector {
    pub fn new(account_id: String) -> Self {
        let base_url = std::env::var("MICROSOFT_API_BASE_URL")
            .unwrap_or_else(|_| "https://graph.microsoft.com".into());
        Self {
            account_id,
            base_url,
        }
    }

    /// Resolves the access token from the OS keychain.
    fn access_token(&self) -> Result<String> {
        let key = format!("microsoft:{}", self.account_id);
        let tokens = keychain::get_tokens(SERVICE_NAME, &key)?.ok_or_else(|| {
            AppError::NotFound(format!("OAuth tokens for account {}", self.account_id))
        })?;
        Ok(tokens.access_token)
    }

    /// Web URL of the OneDrive folder holding an item, falling back to the
    /// item's own page when it has no parent (the drive root).
    pub async fn folder_url(&self, item_id: &str) -> Result<String> {
        #[derive(Deserialize)]
        struct Item {
            #[serde(rename = "webUrl")]
            web_url: Option<String>,
            #[serde(rename = "parentReference")]
            parent_reference: Option<ParentReference>,
        }
        #[derive(Deserialize)]
        struct ParentReference {
            id: Option<String>,
        }

        let token = self.access_token()?;
        let client = reqwest::Client::new();
        let get_item = |id: String| {
            let request = client
                .get(format!("{}/v1.0/me/drive/items/{id}", self.base_url))
                .bearer_auth(&token)
                .query(&[("$select", "webUrl,parentReference")]);
            async move {
                let resp = request
                    .send()
                    .await
                    .map_err(|e| AppError::Other(format!("OneDrive request failed: {e}")))?;
                if !resp.status().is_success() {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    return Err(AppError::Other(format!(
                        "OneDrive Graph API error ({status}): {text}"
                    )));
                }
                resp.json::<Item>()
                    .await
                    .map_err(|e| AppError::Other(format!("Invalid Graph API JSON response: {e}")))
            }
        };

        let item = get_item(item_id.to_string()).await?;
        if let Some(parent_id) = item.parent_reference.and_then(|p| p.id) {
            if let Some(url) = get_item(parent_id).await?.web_url {
                return Ok(url);
            }
        }
        item.web_url
            .ok_or_else(|| AppError::NotFound(format!("web link for OneDrive item {item_id}")))
    }

    /// Fetches changes using the stored delta cursor (if present) or performs initial crawl.
    pub async fn fetch_changes(&self, cursor: Option<&str>) -> Result<CloudSyncResult> {
        let token = self.access_token()?;
        let client = reqwest::Client::new();

        let mut next_url = match cursor {
            Some(c) if !c.trim().is_empty() => c.to_string(),
            _ => format!("{}/v1.0/me/drive/root/delta", self.base_url),
        };

        let mut events = Vec::new();
        let mut final_delta_link = None;

        // Loop through paginated delta pages
        loop {
            let mut headers = HeaderMap::new();
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {token}"))
                    .map_err(|e| AppError::Other(e.to_string()))?,
            );

            let resp = client
                .get(&next_url)
                .headers(headers)
                .send()
                .await
                .map_err(|e| AppError::Other(format!("OneDrive request failed: {e}")))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                return Err(AppError::Other(format!(
                    "OneDrive Graph API error ({status}): {text}"
                )));
            }

            let delta: DeltaResponse = resp
                .json()
                .await
                .map_err(|e| AppError::Other(format!("Invalid Graph API JSON response: {e}")))?;

            for item in delta.value {
                if item.deleted.is_some() {
                    events.push(SourceEvent::Delete {
                        external_id: format!("onedrive:{}", item.id),
                    });
                } else if item.file.is_some() {
                    let modified_at = item
                        .last_modified_date_time
                        .as_deref()
                        .and_then(parse_rfc3339_to_unix);

                    events.push(SourceEvent::Upsert {
                        external_id: format!("onedrive:{}", item.id),
                        title: item.name,
                        mime: item.file.and_then(|f| f.mime_type),
                        modified_at,
                    });
                }
            }

            if let Some(delta_link) = delta.delta_link {
                final_delta_link = Some(delta_link);
                break;
            } else if let Some(next) = delta.next_link {
                next_url = next;
            } else {
                break;
            }
        }

        Ok(CloudSyncResult {
            events,
            new_cursor: final_delta_link,
        })
    }
}

pub fn parse_rfc3339_to_unix(date_str: &str) -> Option<i64> {
    // Basic RFC 3339 / ISO 8601 parsing into unix seconds
    // Example: "2026-10-25T12:00:00Z"
    let parts: Vec<&str> = date_str.split('T').collect();
    if parts.len() != 2 {
        return None;
    }
    let date_parts: Vec<&str> = parts[0].split('-').collect();
    if date_parts.len() != 3 {
        return None;
    }
    let year: i64 = date_parts[0].parse().ok()?;
    let month: i64 = date_parts[1].parse().ok()?;
    let day: i64 = date_parts[2].parse().ok()?;

    let time_clean = parts[1].trim_end_matches('Z');
    let time_parts: Vec<&str> = time_clean.split(':').collect();
    if time_parts.len() < 2 {
        return None;
    }
    let hour: i64 = time_parts[0].parse().ok()?;
    let minute: i64 = time_parts[1].parse().ok()?;
    let second: i64 = time_parts
        .get(2)
        .and_then(|s| s.split('.').next())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    // Approximate days from epoch 1970-01-01
    let mut days = (year - 1970) * 365 + (year - 1969) / 4;
    let days_in_months = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    for &d in days_in_months.iter().take((month - 1) as usize) {
        days += d;
    }
    if month > 2 && year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
        days += 1;
    }
    days += day - 1;

    Some(days * 86400 + hour * 3600 + minute * 60 + second)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rfc3339_date() {
        let date = "2026-10-25T12:00:00Z";
        let secs = parse_rfc3339_to_unix(date).unwrap();
        assert!(secs > 1700000000);
    }

    #[tokio::test]
    async fn fetch_changes_with_mock_server() {
        if std::env::var("MICROSOFT_API_BASE_URL").is_err() {
            return;
        }
        let connector = OneDriveConnector::new("test_account".into());
        let tokens = keychain::AuthTokens {
            access_token: "mock_token".into(),
            refresh_token: None,
            expires_at: None,
            account_id: "test_account".into(),
            account_email: None,
        };
        keychain::save_tokens(SERVICE_NAME, "microsoft:test_account", &tokens).unwrap();

        let res = connector.fetch_changes(None).await.unwrap();
        assert_eq!(res.events.len(), 2);
        assert!(res.new_cursor.is_some());

        let inc = connector
            .fetch_changes(res.new_cursor.as_deref())
            .await
            .unwrap();
        assert_eq!(inc.events.len(), 2);
    }
}
