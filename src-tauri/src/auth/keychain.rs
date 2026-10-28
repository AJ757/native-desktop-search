//! Secure token storage using the OS keychain.
//!
//! Stores OAuth access and refresh tokens securely in:
//! - Windows: Windows Credential Manager
//! - macOS: Keychain Services
//! - Linux: Secret Service (via DBus)
//!
//! Includes an in-memory fallback for headless test / CI environments.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::Result;

pub const SERVICE_NAME: &str = "native-desktop-search";

static MEMORY_STORE: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<i64>,
    pub account_id: String,
    pub account_email: Option<String>,
}

fn with_memory_store<R>(f: impl FnOnce(&mut HashMap<String, String>) -> R) -> R {
    let mut guard = MEMORY_STORE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.is_none() {
        *guard = Some(HashMap::new());
    }
    f(guard.as_mut().unwrap())
}

fn is_headless_or_test() -> bool {
    std::env::var("CI").is_ok() || std::env::var("KEYRING_MOCK").is_ok() || cfg!(test)
}

/// Stores authentication tokens securely for a service and account key.
pub fn save_tokens(service: &str, user_key: &str, tokens: &AuthTokens) -> Result<()> {
    let secret = serde_json::to_string(tokens)?;
    if is_headless_or_test() {
        with_memory_store(|m| m.insert(format!("{service}:{user_key}"), secret));
        return Ok(());
    }

    match keyring::Entry::new(service, user_key) {
        Ok(entry) => match entry.set_password(&secret) {
            Ok(()) => Ok(()),
            Err(e) => {
                tracing::warn!("Keyring write failed ({e}); falling back to memory store");
                with_memory_store(|m| m.insert(format!("{service}:{user_key}"), secret));
                Ok(())
            }
        },
        Err(e) => {
            tracing::warn!("Keyring entry creation failed ({e}); falling back to memory store");
            with_memory_store(|m| m.insert(format!("{service}:{user_key}"), secret));
            Ok(())
        }
    }
}

/// Retrieves stored authentication tokens, if any exist.
pub fn get_tokens(service: &str, user_key: &str) -> Result<Option<AuthTokens>> {
    if is_headless_or_test() {
        let val = with_memory_store(|m| m.get(&format!("{service}:{user_key}")).cloned());
        return match val {
            Some(json) => Ok(Some(serde_json::from_str(&json)?)),
            None => Ok(None),
        };
    }

    match keyring::Entry::new(service, user_key) {
        Ok(entry) => match entry.get_password() {
            Ok(secret) => Ok(Some(serde_json::from_str(&secret)?)),
            Err(keyring::Error::NoEntry) => {
                let fallback =
                    with_memory_store(|m| m.get(&format!("{service}:{user_key}")).cloned());
                match fallback {
                    Some(json) => Ok(Some(serde_json::from_str(&json)?)),
                    None => Ok(None),
                }
            }
            Err(e) => {
                tracing::debug!("Keyring read error ({e}); checking memory store");
                let fallback =
                    with_memory_store(|m| m.get(&format!("{service}:{user_key}")).cloned());
                match fallback {
                    Some(json) => Ok(Some(serde_json::from_str(&json)?)),
                    None => Ok(None),
                }
            }
        },
        Err(_) => {
            let fallback = with_memory_store(|m| m.get(&format!("{service}:{user_key}")).cloned());
            match fallback {
                Some(json) => Ok(Some(serde_json::from_str(&json)?)),
                None => Ok(None),
            }
        }
    }
}

/// Deletes stored authentication tokens.
pub fn delete_tokens(service: &str, user_key: &str) -> Result<()> {
    with_memory_store(|m| m.remove(&format!("{service}:{user_key}")));

    if !is_headless_or_test() {
        if let Ok(entry) = keyring::Entry::new(service, user_key) {
            let _ = entry.delete_credential();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_save_get_and_delete() {
        let tokens = AuthTokens {
            access_token: "access_123".into(),
            refresh_token: Some("refresh_456".into()),
            expires_at: Some(1730000000),
            account_id: "user_1".into(),
            account_email: Some("user@example.com".into()),
        };

        save_tokens(SERVICE_NAME, "test_account", &tokens).unwrap();

        let retrieved = get_tokens(SERVICE_NAME, "test_account").unwrap();
        assert_eq!(retrieved, Some(tokens));

        delete_tokens(SERVICE_NAME, "test_account").unwrap();
        assert_eq!(get_tokens(SERVICE_NAME, "test_account").unwrap(), None);
    }
}
