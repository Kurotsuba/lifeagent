//! `SecretStore` trait, `KeyringSecretStore`, `MemorySecretStore`, per
//! `docs/PHASE1_SPEC.md` §7.3.
//!
//! API keys never touch the database or logs (see `CLAUDE.md` §12); they live in the OS
//! credential store in production, and in memory in tests.

use std::collections::HashMap;
use std::sync::Mutex;

use keyring::Entry;

use crate::error::CoreError;

const SERVICE_NAME: &str = "LifeAgent";

pub trait SecretStore: Send + Sync {
    fn get(&self, provider_id: &str) -> Result<Option<String>, CoreError>;
    fn set(&self, provider_id: &str, key: &str) -> Result<(), CoreError>;
    fn delete(&self, provider_id: &str) -> Result<(), CoreError>;
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

impl SecretStore for MemorySecretStore {
    fn get(&self, provider_id: &str) -> Result<Option<String>, CoreError> {
        Ok(self
            .entries
            .lock()
            .expect("MemorySecretStore mutex poisoned")
            .get(provider_id)
            .cloned())
    }

    fn set(&self, provider_id: &str, key: &str) -> Result<(), CoreError> {
        self.entries
            .lock()
            .expect("MemorySecretStore mutex poisoned")
            .insert(provider_id.to_string(), key.to_string());
        Ok(())
    }

    fn delete(&self, provider_id: &str) -> Result<(), CoreError> {
        self.entries
            .lock()
            .expect("MemorySecretStore mutex poisoned")
            .remove(provider_id);
        Ok(())
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
    fn memory_store_keys_are_isolated_per_provider() {
        let store = MemorySecretStore::new();
        store.set("openai", "a").unwrap();
        store.set("anthropic", "b").unwrap();
        assert_eq!(store.get("openai").unwrap(), Some("a".to_string()));
        assert_eq!(store.get("anthropic").unwrap(), Some("b".to_string()));
    }
}
