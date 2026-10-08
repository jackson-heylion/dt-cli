use super::*;

pub(super) fn directory(root: &Path) -> PathBuf {
    // A separate directory: personal-workflow profile names can never collide with these files.
    root.join("governed")
}
pub(super) fn profile_path(root: &Path, name: &str) -> Result<PathBuf> {
    profile::validate_name(name)?;
    Ok(directory(root).join(format!("{name}.profile.json")))
}
pub(super) fn cache_path(root: &Path, name: &str) -> Result<PathBuf> {
    profile::validate_name(name)?;
    Ok(directory(root).join(format!("{name}.catalog.json")))
}
pub fn profile_exists(root: &Path, name: &str) -> Result<bool> {
    profile_path(root, name)?
        .try_exists()
        .map_err(|_| storage())
}
pub(super) fn identity_key(p: &GovernedProfile) -> String {
    let issuer = format!("{:x}", Sha256::digest(p.issuer.as_bytes()));
    format!(
        "{PROVIDER}/{issuer}/{}/{}/{}/{}/{CLIENT_ID}",
        p.environment, p.system_id, p.subject_id, p.authorization_id
    )
}
pub(super) fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}
/// Opaque server identifiers that end up in local keys and paths.
pub(super) fn valid_handle(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
pub(super) fn rfc3339(value: &str) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    chrono::DateTime::parse_from_rfc3339(value).ok()
}
pub(super) fn read_file(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(storage()),
    }
}
pub(super) fn read_profile(root: &Path, name: &str) -> Result<Option<GovernedProfile>> {
    let Some(bytes) = read_file(&profile_path(root, name)?)? else {
        return Ok(None);
    };
    let p: GovernedProfile = serde_json::from_slice(&bytes).map_err(|_| profile_invalid())?;
    if p.provider != PROVIDER
        || p.credential_key != identity_key(&p)
        || !valid_id(&p.environment)
        || !valid_id(&p.system_id)
        || !valid_id(&p.subject_id)
        || !valid_handle(&p.authorization_id)
        || rfc3339(&p.authorization_expires_at).is_none()
        || rfc3339(&p.access_expires_at).is_none()
    {
        return Err(profile_invalid());
    }
    Ok(Some(p))
}
/// The governed profile, or a provider-mismatch error when the name belongs to the old provider.
pub(super) fn require_profile(rt: &Runtime, name: &str) -> Result<GovernedProfile> {
    if let Some(p) = read_profile(&rt.root, name)? {
        return Ok(p);
    }
    if profile::read(&rt.root, name)?.is_some() {
        return Err(provider_mismatch());
    }
    Err(not_configured())
}
pub(super) fn read_cache(root: &Path, name: &str, p: &GovernedProfile) -> Result<Option<Cache>> {
    let Some(bytes) = read_file(&cache_path(root, name)?)? else {
        return Ok(None);
    };
    let mut c: Cache = serde_json::from_slice(&bytes).map_err(|_| catalog_invalid())?;
    if c.provider != PROVIDER
        || c.issuer != p.issuer
        || c.environment != p.environment
        || c.system_id != p.system_id
        || c.subject_id != p.subject_id
        || c.authorization_id != p.authorization_id
        || rfc3339(&c.cached_at).is_none()
    {
        return Err(catalog_invalid());
    }
    c.validators = validate_operations(&c.operations)?;
    Ok(Some(c))
}
pub(super) fn save_private<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let root = path.parent().ok_or_else(storage)?;
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(root).map_err(|_| storage())?;
    let temp = root.join(format!(".{}.tmp", rand::random::<u64>()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp).map_err(|_| storage())?;
        let bytes = serde_json::to_vec(value).map_err(|_| storage())?;
        file.write_all(&bytes).map_err(|_| storage())?;
        file.sync_all().map_err(|_| storage())?;
        fs::rename(&temp, path).map_err(|_| storage())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
pub(super) fn remove(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(storage()),
    }
}
