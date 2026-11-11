use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub trait TokenStore: Send + Sync + 'static {
    fn load(&self, server_id: &str) -> Result<Option<String>, String>;
    fn save(&self, server_id: &str, token: &str) -> Result<(), String>;
    fn delete(&self, server_id: &str) -> Result<(), String>;
}

pub struct KeyringTokens;

impl KeyringTokens {
    pub const SERVICE: &str = "io.github.dalibor-osu.vcmp";

    fn entry(server_id: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(Self::SERVICE, &format!("server:{server_id}"))
            .map_err(|error| error.to_string())
    }
}

impl TokenStore for KeyringTokens {
    fn load(&self, server_id: &str) -> Result<Option<String>, String> {
        match Self::entry(server_id)?.get_password() {
            Ok(token) => Ok(Some(token)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    fn save(&self, server_id: &str, token: &str) -> Result<(), String> {
        Self::entry(server_id)?
            .set_password(token)
            .map_err(|error| error.to_string())
    }

    fn delete(&self, server_id: &str) -> Result<(), String> {
        match Self::entry(server_id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct MemoryTokens(pub Arc<Mutex<HashMap<String, String>>>);

impl TokenStore for MemoryTokens {
    fn load(&self, server_id: &str) -> Result<Option<String>, String> {
        Ok(self
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(server_id)
            .cloned())
    }

    fn save(&self, server_id: &str, token: &str) -> Result<(), String> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(server_id.into(), token.into());
        Ok(())
    }

    fn delete(&self, server_id: &str) -> Result<(), String> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(server_id);
        Ok(())
    }
}
