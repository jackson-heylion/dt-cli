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
/// Credentials and integrity keys share the existing user-private, atomic file policy.
/// No OS credential API is called, so upgrades and unattended reads cannot trigger a prompt.
pub struct FileStore {
    root: std::path::PathBuf,
}
impl FileStore {
    pub fn new(config_root: &std::path::Path) -> Self {
        Self {
            root: config_root.join("credentials"),
        }
    }
    fn path(&self, key: &str) -> std::path::PathBuf {
        use sha2::{Digest, Sha256};
        self.root
            .join(format!("{:x}.json", Sha256::digest(key.as_bytes())))
    }
    fn read_bytes(&self, key: &str) -> Result<Option<zeroize::Zeroizing<Vec<u8>>>> {
        crate::private_store::read_bytes(&self.path(key)).map_err(|_| storage())
    }
}
impl CredentialStore for FileStore {
    fn read(&self, key: &str) -> Result<Option<Credentials>> {
        self.read_bytes(key)?
            .map(|bytes| serde_json::from_slice(&bytes).map_err(|_| decode_failure()))
            .transpose()
    }
    fn write(&self, key: &str, c: &Credentials) -> Result<()> {
        crate::private_store::write(&self.path(key), c).map_err(|_| storage())
    }
    fn delete(&self, key: &str) -> Result<()> {
        let path = self.path(key);
        if !crate::private_store::validate(&self.root, true).map_err(|_| storage())?
            || !crate::private_store::validate(&path, false).map_err(|_| storage())?
        {
            return Ok(());
        }
        std::fs::remove_file(path).map_err(|_| storage())
    }
    fn read_integrity_key(&self, key: &str) -> Result<Option<Vec<u8>>> {
        self.read_bytes(key)?
            .map(|bytes| serde_json::from_slice::<Vec<u8>>(&bytes).map_err(|_| decode_failure()))
            .transpose()
    }
    fn write_integrity_key(&self, key: &str, bytes: &[u8]) -> Result<()> {
        crate::private_store::write(&self.path(key), &bytes).map_err(|_| storage())
    }
}
