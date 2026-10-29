//! Google Drive connector.
//!
//! The first sync records a Changes API start token and then lists every file.
//! Later syncs pass the stored token to `/drive/v3/changes` to receive only what
//! changed since, much like the OneDrive delta link.

use serde::Deserialize;

use crate::auth::keychain::{self, SERVICE_NAME};
use crate::connectors::onedrive::{parse_rfc3339_to_unix, CloudSyncResult};
use crate::error::{AppError, Result};
use crate::sources::SourceEvent;

const FOLDER_MIME: &str = "application/vnd.google-apps.folder";
const FILE_FIELDS: &str = "id,name,mimeType,modifiedTime,trashed";
/// Files larger than this are indexed by title only.
const MAX_DOWNLOAD_BYTES: u64 = 25 * 1024 * 1024;
/// Binary formats handed to the local text extractor.
const EXTRACTED_EXTENSIONS: &[&str] = &["pdf", "docx", "pptx", "xlsx", "xls", "rtf", "html", "htm"];

/// Base URL for the Drive API. `GOOGLE_API_BASE_URL` points it at a mock
/// server in tests; a blank value counts as unset.
pub fn api_base_url() -> String {
    std::env::var("GOOGLE_API_BASE_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "https://www.googleapis.com".into())
}

#[derive(Debug, Clone)]
pub struct GoogleDriveConnector {
    account_id: String,
    base_url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartPageToken {
    start_page_token: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileList {
    next_page_token: Option<String>,
    #[serde(default)]
    files: Vec<DriveFile>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChangeList {
    next_page_token: Option<String>,
    new_start_page_token: Option<String>,
    #[serde(default)]
    changes: Vec<Change>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Change {
    file_id: String,
    #[serde(default)]
    removed: bool,
    file: Option<DriveFile>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DriveFile {
    id: String,
    #[serde(default)]
    name: String,
    mime_type: Option<String>,
    modified_time: Option<String>,
    #[serde(default)]
    trashed: bool,
}

/// Export format for Google Docs editor files, which have no raw bytes.
fn export_mime(mime: &str) -> Option<&'static str> {
    match mime {
        "application/vnd.google-apps.document" => Some("text/plain"),
        "application/vnd.google-apps.spreadsheet" => Some("text/csv"),
        "application/vnd.google-apps.presentation" => Some("text/plain"),
        _ => None,
    }
}

/// Turns downloaded bytes into text. Formats the local extractor understands
/// (PDF, Office, HTML, RTF) go through it via a temporary file; anything else
/// is used as UTF-8 text unless it looks binary.
fn bytes_to_text(file_id: &str, title: &str, bytes: &[u8]) -> Result<String> {
    let ext = std::path::Path::new(title)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);

    if let Some(ext) = ext.filter(|e| EXTRACTED_EXTENSIONS.contains(&e.as_str())) {
        let tmp = std::env::temp_dir().join(format!("nds-gdrive-{file_id}.{ext}"));
        std::fs::write(&tmp, bytes)?;
        let result = crate::extract::extract(&tmp);
        let _ = std::fs::remove_file(&tmp);
        return Ok(result?.text);
    }

    if bytes.iter().take(8192).any(|&b| b == 0) {
        return Ok(String::new());
    }
    Ok(String::from_utf8_lossy(bytes).into_owned())
}

fn external_id(file_id: &str) -> String {
    format!("gdrive:{file_id}")
}

fn upsert_event(file: DriveFile) -> SourceEvent {
    SourceEvent::Upsert {
        external_id: external_id(&file.id),
        modified_at: file
            .modified_time
            .as_deref()
            .and_then(parse_rfc3339_to_unix),
        title: file.name,
        mime: file.mime_type,
    }
}

impl GoogleDriveConnector {
    pub fn new(account_id: String) -> Self {
        Self {
            account_id,
            base_url: api_base_url(),
        }
    }

    /// Resolves the access token from the OS keychain, refreshing it first
    /// when it expires within a minute.
    async fn access_token(&self) -> Result<String> {
        let key = format!("google:{}", self.account_id);
        let tokens = keychain::get_tokens(SERVICE_NAME, &key)?.ok_or_else(|| {
            AppError::NotFound(format!(
                "OAuth tokens for Google account {}",
                self.account_id
            ))
        })?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let expiring = tokens.expires_at.is_some_and(|at| at <= now + 60);

        if expiring && tokens.refresh_token.is_some() {
            let fresh = crate::auth::oauth::refresh_google_tokens(&tokens).await?;
            keychain::save_tokens(SERVICE_NAME, &key, &fresh)?;
            tracing::info!(account_id = %self.account_id, "Google access token refreshed");
            return Ok(fresh.access_token);
        }
        Ok(tokens.access_token)
    }

    /// Downloads a file's text for indexing. Google Docs, Sheets and Slides
    /// are exported as plain text or CSV; other Google-native types (forms,
    /// drawings, shortcuts) and files over the size limit yield empty text.
    pub async fn download_text(
        &self,
        file_id: &str,
        title: &str,
        mime: Option<&str>,
    ) -> Result<String> {
        let mime = mime.unwrap_or("");
        let request = match export_mime(mime) {
            Some(export) => self
                .client_get(&format!("/drive/v3/files/{file_id}/export"))
                .query(&[("mimeType", export)]),
            None if mime.starts_with("application/vnd.google-apps.") => {
                return Ok(String::new());
            }
            None => self
                .client_get(&format!("/drive/v3/files/{file_id}"))
                .query(&[("alt", "media")]),
        };

        let token = self.access_token().await?;
        let resp = request
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| AppError::Other(format!("Google Drive download failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AppError::Other(format!(
                "Google Drive download error ({status}): {text}"
            )));
        }
        if resp
            .content_length()
            .is_some_and(|n| n > MAX_DOWNLOAD_BYTES)
        {
            tracing::info!(file_id, "Google Drive file too large to index; title only");
            return Ok(String::new());
        }

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| AppError::Other(format!("Google Drive download failed: {e}")))?;
        bytes_to_text(file_id, title, &bytes)
    }

    fn client_get(&self, path: &str) -> reqwest::RequestBuilder {
        reqwest::Client::new().get(format!("{}{path}", self.base_url))
    }

    /// Web URL of the Drive folder holding a file. Files shared with the user
    /// have no visible parent, so the file's own page is returned instead.
    pub async fn folder_url(&self, file_id: &str) -> Result<String> {
        #[derive(Deserialize)]
        struct Parents {
            #[serde(default)]
            parents: Vec<String>,
        }

        let token = self.access_token().await?;
        let client = reqwest::Client::new();
        let file: Parents = self
            .get_json(
                &client,
                &token,
                &format!("/drive/v3/files/{file_id}"),
                &[("fields", "parents")],
            )
            .await?;

        Ok(match file.parents.first() {
            Some(parent) => format!("https://drive.google.com/drive/folders/{parent}"),
            None => format!("https://drive.google.com/file/d/{file_id}/view"),
        })
    }

    async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        client: &reqwest::Client,
        token: &str,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<T> {
        let resp = client
            .get(format!("{}{path}", self.base_url))
            .bearer_auth(token)
            .query(query)
            .send()
            .await
            .map_err(|e| AppError::Other(format!("Google Drive request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AppError::Other(format!(
                "Google Drive API error ({status}): {text}"
            )));
        }

        resp.json()
            .await
            .map_err(|e| AppError::Other(format!("Invalid Google Drive JSON response: {e}")))
    }

    /// Fetches changes since `cursor` (a Changes API page token), or lists
    /// every file when there is no cursor yet.
    pub async fn fetch_changes(&self, cursor: Option<&str>) -> Result<CloudSyncResult> {
        let token = self.access_token().await?;
        let client = reqwest::Client::new();

        match cursor {
            Some(c) if !c.trim().is_empty() => self.incremental_sync(&client, &token, c).await,
            _ => self.initial_crawl(&client, &token).await,
        }
    }

    async fn initial_crawl(
        &self,
        client: &reqwest::Client,
        token: &str,
    ) -> Result<CloudSyncResult> {
        // Take the start token first so nothing changed during the listing is missed.
        let start: StartPageToken = self
            .get_json(client, token, "/drive/v3/changes/startPageToken", &[])
            .await?;

        let fields = format!("nextPageToken,files({FILE_FIELDS})");
        let mut events = Vec::new();
        let mut page_token: Option<String> = None;

        loop {
            let mut query = vec![
                ("q", "trashed = false"),
                ("pageSize", "1000"),
                ("fields", fields.as_str()),
                ("spaces", "drive"),
            ];
            if let Some(pt) = page_token.as_deref() {
                query.push(("pageToken", pt));
            }

            let list: FileList = self
                .get_json(client, token, "/drive/v3/files", &query)
                .await?;

            events.extend(
                list.files
                    .into_iter()
                    .filter(|f| !f.trashed && f.mime_type.as_deref() != Some(FOLDER_MIME))
                    .map(upsert_event),
            );

            match list.next_page_token {
                Some(next) => page_token = Some(next),
                None => break,
            }
        }

        Ok(CloudSyncResult {
            events,
            new_cursor: Some(start.start_page_token),
        })
    }

    async fn incremental_sync(
        &self,
        client: &reqwest::Client,
        token: &str,
        cursor: &str,
    ) -> Result<CloudSyncResult> {
        let fields =
            format!("nextPageToken,newStartPageToken,changes(fileId,removed,file({FILE_FIELDS}))");
        let mut events = Vec::new();
        let mut page_token = cursor.to_string();

        loop {
            let query = [
                ("pageToken", page_token.as_str()),
                ("pageSize", "1000"),
                ("fields", fields.as_str()),
                ("spaces", "drive"),
            ];

            let list: ChangeList = self
                .get_json(client, token, "/drive/v3/changes", &query)
                .await?;

            for change in list.changes {
                match change.file {
                    Some(file) if !change.removed && !file.trashed => {
                        if file.mime_type.as_deref() != Some(FOLDER_MIME) {
                            events.push(upsert_event(file));
                        }
                    }
                    _ => events.push(SourceEvent::Delete {
                        external_id: external_id(&change.file_id),
                    }),
                }
            }

            if let Some(new_start) = list.new_start_page_token {
                return Ok(CloudSyncResult {
                    events,
                    new_cursor: Some(new_start),
                });
            }
            match list.next_page_token {
                Some(next) => page_token = next,
                None => {
                    return Ok(CloudSyncResult {
                        events,
                        new_cursor: Some(page_token),
                    })
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_list_parses_removed_and_trashed_files() {
        let json = r#"{
            "newStartPageToken": "42",
            "changes": [
                {"fileId": "a", "removed": true},
                {"fileId": "b", "file": {"id": "b", "name": "b.txt", "trashed": true}},
                {"fileId": "c", "file": {"id": "c", "name": "c.txt", "mimeType": "text/plain",
                 "modifiedTime": "2026-10-25T12:00:00.000Z"}}
            ]
        }"#;
        let list: ChangeList = serde_json::from_str(json).unwrap();
        assert_eq!(list.new_start_page_token.as_deref(), Some("42"));
        assert!(list.changes[0].removed);
        assert!(list.changes[1].file.as_ref().unwrap().trashed);
        assert!(list.changes[2]
            .file
            .as_ref()
            .unwrap()
            .modified_time
            .is_some());
    }

    #[tokio::test]
    async fn fetch_changes_with_mock_server() {
        if std::env::var("GOOGLE_API_BASE_URL").is_err() {
            return;
        }
        let connector = GoogleDriveConnector::new("test_account".into());
        let tokens = keychain::AuthTokens {
            access_token: "mock_token".into(),
            refresh_token: None,
            expires_at: None,
            account_id: "test_account".into(),
            account_email: None,
        };
        keychain::save_tokens(SERVICE_NAME, "google:test_account", &tokens).unwrap();

        // Initial crawl: two files, the folder is skipped.
        let res = connector.fetch_changes(None).await.unwrap();
        assert_eq!(res.events.len(), 2);
        assert_eq!(res.new_cursor.as_deref(), Some("start_1"));

        // Incremental: one modified, one deleted.
        let inc = connector
            .fetch_changes(res.new_cursor.as_deref())
            .await
            .unwrap();
        assert_eq!(inc.events.len(), 2);
        assert!(matches!(inc.events[1], SourceEvent::Delete { .. }));
        assert_eq!(inc.new_cursor.as_deref(), Some("start_2"));

        // Content: a plain file is downloaded, a Google Doc is exported.
        let md = connector
            .download_text(
                "gdrive_file_1",
                "Terraform_Modules.md",
                Some("text/markdown"),
            )
            .await
            .unwrap();
        assert!(md.contains("Terraform module layout"));

        let doc = connector
            .download_text(
                "gdrive_file_2",
                "Release_Checklist",
                Some("application/vnd.google-apps.document"),
            )
            .await
            .unwrap();
        assert!(doc.contains("release checklist"));

        let form = connector
            .download_text("f", "Survey", Some("application/vnd.google-apps.form"))
            .await
            .unwrap();
        assert!(form.is_empty());

        // The containing folder's web page.
        assert_eq!(
            connector.folder_url("gdrive_file_1").await.unwrap(),
            "https://drive.google.com/drive/folders/gdrive_folder_1"
        );
    }

    #[test]
    fn binary_bytes_are_not_treated_as_text() {
        assert_eq!(bytes_to_text("x", "a.bin", b"ab\0cd").unwrap(), "");
        assert_eq!(
            bytes_to_text("x", "a.csv", b"a,b\n1,2").unwrap(),
            "a,b\n1,2"
        );
    }
}
