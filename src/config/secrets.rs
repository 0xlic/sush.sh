#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::sync::Mutex;

type SecretResult<T> = std::result::Result<T, SecretError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretKind {
    LoginPassword,
    KeyPassphrase,
}

impl SecretKind {
    fn as_account_part(self) -> &'static str {
        match self {
            Self::LoginPassword => "login_password",
            Self::KeyPassphrase => "key_passphrase",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretKey {
    pub service: String,
    pub account: String,
}

impl SecretKey {
    pub fn new(host_id: &str, kind: SecretKind, identity_hint: Option<&str>) -> Self {
        let service = "sush".to_string();
        let account = match identity_hint {
            Some(identity_hint) if kind == SecretKind::KeyPassphrase => {
                format!("{host_id}:{}:{identity_hint}", kind.as_account_part())
            }
            _ => format!("{host_id}:{}", kind.as_account_part()),
        };

        Self { service, account }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretError {
    Unavailable(String),
    PermissionDenied(String),
    Backend(String),
}

impl std::fmt::Display for SecretError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(msg) | Self::PermissionDenied(msg) | Self::Backend(msg) => {
                f.write_str(msg)
            }
        }
    }
}

impl std::error::Error for SecretError {}

impl SecretError {
    pub fn user_message(&self) -> &'static str {
        match self {
            Self::Unavailable(_) => "system keyring is unavailable",
            Self::PermissionDenied(_) => "permission denied by system keyring",
            Self::Backend(_) => "failed to access system keyring",
        }
    }
}

pub trait SecretBackend: Send + Sync {
    fn is_available(&self) -> bool;
    fn get(&self, key: &SecretKey) -> SecretResult<Option<String>>;
    fn set(&self, key: &SecretKey, value: &str) -> SecretResult<()>;
    fn delete(&self, key: &SecretKey) -> SecretResult<()>;
}

pub struct SecretStore {
    backend: Box<dyn SecretBackend>,
}

impl SecretStore {
    pub fn new(backend: Box<dyn SecretBackend>) -> Self {
        Self { backend }
    }

    #[allow(dead_code)]
    pub fn is_available(&self) -> bool {
        self.backend.is_available()
    }

    pub fn get(&self, key: &SecretKey) -> SecretResult<Option<String>> {
        self.backend.get(key)
    }

    pub fn set(&self, key: &SecretKey, value: &str) -> SecretResult<()> {
        self.backend.set(key, value)
    }

    #[allow(dead_code)]
    pub fn delete(&self, key: &SecretKey) -> SecretResult<()> {
        self.backend.delete(key)
    }
}

pub struct SystemSecretBackend;

impl SystemSecretBackend {
    pub fn new() -> Self {
        Self
    }

    fn entry(&self, key: &SecretKey) -> SecretResult<keyring::Entry> {
        keyring::Entry::new(&key.service, &key.account).map_err(map_keyring_error)
    }
}

impl Default for SystemSecretBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretBackend for SystemSecretBackend {
    fn is_available(&self) -> bool {
        let probe_key = SecretKey {
            service: "sush".into(),
            account: "__availability_probe__".into(),
        };
        self.get(&probe_key).is_ok()
    }

    fn get(&self, key: &SecretKey) -> SecretResult<Option<String>> {
        let entry = self.entry(key)?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(map_keyring_error(err)),
        }
    }

    fn set(&self, key: &SecretKey, value: &str) -> SecretResult<()> {
        let entry = self.entry(key)?;
        entry.set_password(value).map_err(map_keyring_error)
    }

    fn delete(&self, key: &SecretKey) -> SecretResult<()> {
        let entry = self.entry(key)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(map_keyring_error(err)),
        }
    }
}

fn map_keyring_error(err: keyring::Error) -> SecretError {
    match err {
        keyring::Error::NoStorageAccess(inner) => classify_storage_message(inner.to_string()),
        keyring::Error::PlatformFailure(inner) => classify_storage_message(inner.to_string()),
        keyring::Error::BadEncoding(_) => {
            SecretError::Backend("stored secret is not valid UTF-8".into())
        }
        keyring::Error::TooLong(attr, limit) => {
            SecretError::Backend(format!("attribute {attr} exceeds platform limit {limit}"))
        }
        keyring::Error::Invalid(attr, reason) => {
            SecretError::Backend(format!("invalid attribute {attr}: {reason}"))
        }
        keyring::Error::Ambiguous(_) => {
            SecretError::Backend("multiple matching entries found in system keyring".into())
        }
        keyring::Error::NoEntry => SecretError::Backend("no matching keyring entry".into()),
        _ => SecretError::Backend(err.to_string()),
    }
}

fn classify_storage_message(message: String) -> SecretError {
    let lowercase = message.to_lowercase();
    if lowercase.contains("permission denied")
        || lowercase.contains("access denied")
        || lowercase.contains("not permitted")
        || lowercase.contains("unauthorized")
    {
        return SecretError::PermissionDenied(message);
    }

    if lowercase.contains("secret service")
        || lowercase.contains("org.freedesktop.secrets")
        || lowercase.contains("dbus")
        || lowercase.contains("keyring is locked")
        || lowercase.contains("no such secret collection")
        || lowercase.contains("cannot spawn a message bus")
        || lowercase.contains("no storage access")
    {
        return SecretError::Unavailable(message);
    }

    SecretError::Backend(message)
}

#[cfg(test)]
pub struct FakeBackend {
    available: bool,
    entries: Mutex<HashMap<String, String>>,
}

#[cfg(test)]
impl FakeBackend {
    pub fn available() -> Self {
        Self {
            available: true,
            entries: Mutex::new(HashMap::new()),
        }
    }

    pub fn unavailable() -> Self {
        Self {
            available: false,
            entries: Mutex::new(HashMap::new()),
        }
    }

    fn entry_name(key: &SecretKey) -> String {
        format!("{}::{}", key.service, key.account)
    }
}

#[cfg(test)]
impl SecretBackend for FakeBackend {
    fn is_available(&self) -> bool {
        self.available
    }

    fn get(&self, key: &SecretKey) -> SecretResult<Option<String>> {
        if !self.available {
            return Err(SecretError::Unavailable("backend unavailable".into()));
        }

        let entries = self
            .entries
            .lock()
            .map_err(|_| SecretError::Backend("backend state poisoned".into()))?;
        Ok(entries.get(&Self::entry_name(key)).cloned())
    }

    fn set(&self, key: &SecretKey, value: &str) -> SecretResult<()> {
        if !self.available {
            return Err(SecretError::Unavailable("backend unavailable".into()));
        }

        let mut entries = self
            .entries
            .lock()
            .map_err(|_| SecretError::Backend("backend state poisoned".into()))?;
        entries.insert(Self::entry_name(key), value.to_string());
        Ok(())
    }

    fn delete(&self, key: &SecretKey) -> SecretResult<()> {
        if !self.available {
            return Err(SecretError::Unavailable("backend unavailable".into()));
        }

        let mut entries = self
            .entries
            .lock()
            .map_err(|_| SecretError::Backend("backend state poisoned".into()))?;
        entries.remove(&Self::entry_name(key));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_key_is_stable() {
        let key = SecretKey::new("host-1", SecretKind::LoginPassword, None);
        assert_eq!(key.service, "sush");
        assert_eq!(key.account, "host-1:login_password");
    }

    #[test]
    fn key_passphrase_key_includes_identity_path() {
        let key = SecretKey::new(
            "host-1",
            SecretKind::KeyPassphrase,
            Some("/Users/me/.ssh/id_ed25519"),
        );
        assert!(key.account.contains("host-1:key_passphrase:"));
    }

    #[test]
    fn fake_backend_roundtrip() {
        let store = SecretStore::new(Box::new(FakeBackend::available()));
        let key = SecretKey::new("host-1", SecretKind::LoginPassword, None);
        store.set(&key, "secret").unwrap();
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("secret"));
    }

    #[test]
    fn unavailable_backend_rejects_persist() {
        let store = SecretStore::new(Box::new(FakeBackend::unavailable()));
        let key = SecretKey::new("host-1", SecretKind::LoginPassword, None);
        let err = store.set(&key, "secret").unwrap_err();
        assert!(matches!(err, SecretError::Unavailable(_)));
    }

    #[test]
    fn unavailable_error_is_user_readable() {
        let err = SecretError::Unavailable("linux secret service not found".into());
        assert_eq!(err.user_message(), "system keyring is unavailable");
    }

    #[test]
    fn permission_error_is_user_readable() {
        let err = SecretError::PermissionDenied("access denied".into());
        assert_eq!(err.user_message(), "permission denied by system keyring");
    }
}
