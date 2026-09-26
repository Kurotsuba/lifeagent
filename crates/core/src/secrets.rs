//! `SecretStore` trait, `KeyringSecretStore`, `MemorySecretStore`, per
//! `docs/PHASE1_SPEC.md` §7.3.
//!
//! API keys never touch the database or logs (see `CLAUDE.md` §12); they live in the OS
//! credential store in production, and in memory in tests.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use keyring::Entry;
use lifeagent_llm::error::LlmError;
use lifeagent_llm::router::KeyProvider;

use crate::error::CoreError;

const SERVICE_NAME: &str = "LifeAgent";

pub trait SecretStore: Send + Sync {
    fn get(&self, provider_id: &str) -> Result<Option<String>, CoreError>;
    fn set(&self, provider_id: &str, key: &str) -> Result<(), CoreError>;
    fn delete(&self, provider_id: &str) -> Result<(), CoreError>;

    /// Whether a key is set, without handing the key value itself back to the caller (§10.2's
    /// `has_api_key`: Settings shows presence, never the key). Default-implemented in terms of
    /// `get` so neither impl below needs to change.
    fn has(&self, provider_id: &str) -> Result<bool, CoreError> {
        Ok(self.get(provider_id)?.is_some())
    }
}

/// Stores API keys in the OS credential store (Windows Credential Manager, macOS Keychain,
/// the Secret Service on Linux), account `provider:{id}` under service `LifeAgent`.
pub struct KeyringSecretStore;

impl KeyringSecretStore {
    fn entry(provider_id: &str) -> Result<Entry, CoreError> {
        Entry::new(SERVICE_NAME, &format!("provider:{provider_id}"))
            .map_err(|e| CoreError::Secret(e.to_string()))
    }
}

impl SecretStore for KeyringSecretStore {
    fn get(&self, provider_id: &str) -> Result<Option<String>, CoreError> {
        match Self::entry(provider_id)?.get_password() {
            Ok(password) => Ok(Some(password)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(CoreError::Secret(e.to_string())),
        }
    }

    fn set(&self, provider_id: &str, key: &str) -> Result<(), CoreError> {
        Self::entry(provider_id)?
            .set_password(key)
            .map_err(|e| CoreError::Secret(e.to_string()))
    }

    fn delete(&self, provider_id: &str) -> Result<(), CoreError> {
        match Self::entry(provider_id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(CoreError::Secret(e.to_string())),
        }
    }
}

/// In-memory `SecretStore` for tests — never touches the OS credential store.
#[derive(Default)]
pub struct MemorySecretStore {
    entries: Mutex<HashMap<String, String>>,
}

impl MemorySecretStore {
    pub fn new() -> Self {
        Self::default()
    }
}

// A poisoned lock is recovered rather than propagated: each operation is one `HashMap` call,
// so a panicking holder can't leave the map half-updated.
impl SecretStore for MemorySecretStore {
    fn get(&self, provider_id: &str) -> Result<Option<String>, CoreError> {
        Ok(self
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(provider_id)
            .cloned())
    }

    fn set(&self, provider_id: &str, key: &str) -> Result<(), CoreError> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(provider_id.to_string(), key.to_string());
        Ok(())
    }

    fn delete(&self, provider_id: &str) -> Result<(), CoreError> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(provider_id);
        Ok(())
    }
}

/// Adapts any [`SecretStore`] to the LLM layer's `KeyProvider` trait (§6.3), so
/// `lifeagent-llm` stays decoupled from how keys are stored (`CoreApp::build`, M3).
pub struct SecretKeyProvider(pub Arc<dyn SecretStore>);

impl KeyProvider for SecretKeyProvider {
    fn get_key(&self, provider_id: &str) -> Result<String, LlmError> {
        self.0
            .get(provider_id)
            .map_err(|e| LlmError::Config(e.to_string()))?
            .ok_or_else(|| LlmError::Config(format!("no API key set for provider {provider_id:?}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_get_is_none_when_unset() {
        let store = MemorySecretStore::new();
        assert_eq!(store.get("openai").unwrap(), None);
    }

    #[test]
    fn memory_store_set_then_get_round_trips() {
        let store = MemorySecretStore::new();
        store.set("openai", "sk-test-123").unwrap();
        assert_eq!(
            store.get("openai").unwrap(),
            Some("sk-test-123".to_string())
        );
    }

    #[test]
    fn memory_store_set_overwrites_previous_value() {
        let store = MemorySecretStore::new();
        store.set("openai", "first").unwrap();
        store.set("openai", "second").unwrap();
        assert_eq!(store.get("openai").unwrap(), Some("second".to_string()));
    }

    #[test]
    fn memory_store_delete_removes_the_key() {
        let store = MemorySecretStore::new();
        store.set("openai", "sk-test-123").unwrap();
        store.delete("openai").unwrap();
        assert_eq!(store.get("openai").unwrap(), None);
    }

    #[test]
    fn memory_store_delete_of_missing_key_is_not_an_error() {
        let store = MemorySecretStore::new();
        assert!(store.delete("never-set").is_ok());
    }

    #[test]
    fn has_reports_presence_without_the_default_impl_leaking_the_key() {
        let store = MemorySecretStore::new();
        assert!(!store.has("openai").unwrap());
        store.set("openai", "sk-test").unwrap();
        assert!(store.has("openai").unwrap());
    }

    #[test]
    fn memory_store_keys_are_isolated_per_provider() {
        let store = MemorySecretStore::new();
        store.set("openai", "a").unwrap();
        store.set("anthropic", "b").unwrap();
        assert_eq!(store.get("openai").unwrap(), Some("a".to_string()));
        assert_eq!(store.get("anthropic").unwrap(), Some("b".to_string()));
    }

    #[test]
    fn key_provider_returns_the_stored_key() {
        let store: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::new());
        store.set("openai", "sk-test").unwrap();
        let provider = SecretKeyProvider(store);
        assert_eq!(provider.get_key("openai").unwrap(), "sk-test");
    }

    #[test]
    fn key_provider_errors_when_no_key_is_set() {
        let store: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::new());
        let provider = SecretKeyProvider(store);
        assert!(matches!(
            provider.get_key("openai"),
            Err(LlmError::Config(_))
        ));
    }
}
