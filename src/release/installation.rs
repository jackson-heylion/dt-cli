use super::*;

pub(super) fn default_root(candidate: bool) -> Result<PathBuf> {
    directories::ProjectDirs::from("com", "datousoft", "dt-cli")
        .map(|p| {
            p.data_local_dir().join(if candidate {
                "candidate-installation"
            } else {
                "installation"
            })
        })
        .ok_or_else(io_error)
}
pub(super) fn running_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let parent = exe.parent()?;
    if parent.join(MARKER).is_file() {
        return parent.parent().map(Path::to_owned);
    }
    if parent.parent()?.file_name()? == "versions" {
        let root = parent.parent()?.parent()?;
        if root.join("trust.json").is_file() {
            return Some(root.to_owned());
        }
    }
    None
}
pub(super) fn manifest_at(
    root: &Path,
    key: &str,
    host: (&str, &str),
) -> Result<(Manifest, PathBuf)> {
    if !digest(key) {
        return Err(io_error());
    }
    let folder = root.join("versions").join(key);
    if fs::symlink_metadata(&folder)
        .map_err(|_| io_error())?
        .file_type()
        .is_symlink()
    {
        return Err(io_error());
    }
    let manifest: Manifest =
        serde_json::from_slice(&read_limited(&folder.join("manifest.json"), MAX_JSON)?)
            .map_err(|_| bad_package())?;
    manifest.validate(host)?;
    let binary = folder.join(&manifest.binary);
    if manifest.sha256 != key || sha(&read_limited(&binary, MAX_BINARY)?) != key {
        return Err(bad_package());
    }
    Ok((manifest, binary))
}
pub(super) fn trust_at(root: &Path) -> Result<Trust> {
    let trust: Trust = serde_json::from_slice(&read_limited(&root.join("trust.json"), MAX_JSON)?)
        .map_err(|_| io_error())?;
    if trust.schema_version != SCHEMA
        || !matches!(trust.release_type.as_str(), "candidate" | "release")
        || (trust.release_type == "candidate" && trust.identity.is_some())
    {
        return Err(io_error());
    }
    Ok(trust)
}
pub(super) fn verify_payload(
    manifest: &Manifest,
    binary: &Path,
    trust: &Trust,
    verifier: &dyn Verifier,
) -> Result<()> {
    if manifest.release_type != trust.release_type {
        return Err(failure(
            "RELEASE_CHANNEL_MISMATCH",
            "候选与正式安装目录相互独立；不能切换安装的信任方式。",
        ));
    }
    manifest.matches_probe(&verifier.probe(binary)?)
}
pub(super) fn status(root: &Path, host: (&str, &str)) -> Result<Value> {
    if !root.join("trust.json").exists() {
        return Ok(
            json!({"managed":false,"installationDirectory":root,"updateAvailable":null,"checkSource":"local","httpRequests":0}),
        );
    }
    let trust = trust_at(root)?;
    let selected = Selection::read(root)?;
    let (current, _) = manifest_at(root, &selected.current, host)?;
    let previous = selected
        .previous
        .as_deref()
        .map(|s| manifest_at(root, s, host).map(|(m, _)| m))
        .transpose()?;
    Ok(
        json!({"managed":true,"installationDirectory":root,"launcher":root.join("bin").join(binary_name(host.0)),"releaseType":trust.release_type,"current":current,"previous":previous,"updateAvailable":null,"checkSource":"local","httpRequests":0,"automaticUpdate":false}),
    )
}
pub(super) fn prune(
    root: &Path,
    selected: &Selection,
    host: (&str, &str),
    check: bool,
) -> Result<Value> {
    let mut removed = Vec::new();
    let mut retained = Vec::new();
    let mut eligible = Vec::new();
    for entry in fs::read_dir(root.join("versions")).map_err(|_| io_error())? {
        let entry = entry.map_err(|_| io_error())?;
        let key = entry.file_name().to_string_lossy().into_owned();
        if !digest(&key)
            || key == selected.current
            || selected.previous.as_deref() == Some(key.as_str())
        {
            continue;
        }
        let Ok((manifest, binary)) = manifest_at(root, &key, host) else {
            retained.push(key);
            continue;
        };
        let files = fs::read_dir(entry.path())
            .map_err(|_| io_error())?
            .collect::<std::io::Result<Vec<_>>>()
            .map_err(|_| io_error())?;
        if files.len() != 2
            || files.iter().any(|e| {
                e.file_name() != manifest.binary.as_str() && e.file_name() != "manifest.json"
            })
        {
            retained.push(key);
            continue;
        }
        if check {
            eligible.push(key);
            continue;
        }
        // Delete the executable first. Windows refuses an in-use EXE; its manifest then remains intact for a later retry.
        if fs::remove_file(&binary).is_err() {
            retained.push(key);
            continue;
        }
        if fs::remove_file(entry.path().join("manifest.json"))
            .and_then(|_| fs::remove_dir(entry.path()))
            .is_err()
        {
            retained.push(key);
        } else {
            removed.push(key);
        }
    }
    Ok(
        json!({"action":if check{"prune-check"}else{"prune"},"removed":removed,"eligible":eligible,"retained":retained,"currentAndPreviousRetained":true,"configurationChanged":false,"credentialsChanged":false,"skillsChanged":false}),
    )
}
pub(super) struct Options<'a> {
    pub(super) install: bool,
    pub(super) check: bool,
    pub(super) rollback: bool,
    pub(super) prune: bool,
    pub(super) candidate: bool,
    pub(super) expected: Option<&'a str>,
    pub(super) signer: Option<&'a str>,
}
