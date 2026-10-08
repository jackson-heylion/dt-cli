use crate::output::{Failure, Result, storage};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(deny_unknown_fields)]
pub struct Credentials {
    pub access_token: String,
    pub refresh_token: String,
    #[serde(default = "version")]
    pub version: u8,
    #[serde(default)]
    pub access_expires_at: Option<String>,
    #[serde(default)]
    pub authorization_expires_at: Option<String>,
    #[serde(default)]
    pub generation: u64,
    #[serde(default = "ready")]
    pub refresh_state: String,
    /// A random, durable nonce for the in-flight rotation; omitted once it completes so the
    /// normal v1 record remains readable by previous releases.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_request_id: Option<String>,
}
fn version() -> u8 {
    1
}
fn ready() -> String {
    "ready".into()
}
pub trait CredentialStore: Send + Sync {
    fn read(&self, key: &str) -> Result<Option<Credentials>>;
    fn write(&self, key: &str, c: &Credentials) -> Result<()>;
    fn delete(&self, key: &str) -> Result<()>;
    /// Separate opaque key material for local metadata integrity, never an OAuth credential.
    fn read_integrity_key(&self, _key: &str) -> Result<Option<Vec<u8>>> {
        Err(storage())
    }
    fn write_integrity_key(&self, _key: &str, _bytes: &[u8]) -> Result<()> {
        Err(storage())
    }
}
fn decode_failure() -> Failure {
    Failure::new(
        "CREDENTIAL_DECODE_FAILED",
        1,
        "安全存储中的凭证记录无法解码；原记录已保留。",
    )
}
pub struct SystemStore;
impl SystemStore {
    fn with_entry<T>(
        &self,
        key: &str,
        operation: impl FnOnce(keyring::Entry) -> Result<T>,
    ) -> Result<T> {
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = (key, operation);
            Err(storage())
        }
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            // Keychain interaction mode is process-wide. Serialize the scope so one operation
            // cannot restore prompting while another still needs a noninteractive lookup.
            #[cfg(target_os = "macos")]
            static KEYCHAIN: std::sync::Mutex<()> = std::sync::Mutex::new(());
            #[cfg(target_os = "macos")]
            let _serial = KEYCHAIN.lock().map_err(|_| storage())?;
            #[cfg(target_os = "macos")]
            let _interaction =
                security_framework::os::macos::keychain::SecKeychain::disable_user_interaction()
                    .map_err(|_| storage())?;
            operation(keyring::Entry::new("com.datousoft.dt-cli", key).map_err(|_| storage())?)
        }
    }
}
impl CredentialStore for SystemStore {
    fn read_integrity_key(&self, key: &str) -> Result<Option<Vec<u8>>> {
        self.with_entry(key, |entry| match entry.get_secret() {
            Ok(bytes) => Ok(Some(bytes)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(storage()),
        })
    }
    fn write_integrity_key(&self, key: &str, bytes: &[u8]) -> Result<()> {
        self.with_entry(key, |entry| entry.set_secret(bytes).map_err(|_| storage()))
    }
    fn read(&self, key: &str) -> Result<Option<Credentials>> {
        self.with_entry(key, |entry| match entry.get_password() {
            Ok(mut s) => {
                let result = serde_json::from_str(&s)
                    .map(Some)
                    .map_err(|_| decode_failure());
                s.zeroize();
                result
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(keyring::Error::BadEncoding(mut bytes)) => {
                bytes.zeroize();
                Err(decode_failure())
            }
            Err(_) => Err(storage()),
        })
    }
    fn write(&self, key: &str, c: &Credentials) -> Result<()> {
        self.with_entry(key, |entry| {
            let mut s = serde_json::to_string(c).map_err(|_| storage())?;
            let result = entry.set_password(&s).map_err(|_| storage());
            s.zeroize();
            result
        })
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.with_entry(key, |entry| match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(storage()),
        })
    }
}
