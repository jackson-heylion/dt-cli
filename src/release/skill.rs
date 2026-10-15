//! Universal Skill installation uses the same fixed HTTPS source as native updates.
use super::*;
use std::collections::BTreeMap;

const LIMIT: u64 = crate::constants::distribution::MAX_SKILL_BYTES;
const RECORD: &str = crate::constants::distribution::SKILL_RECORD;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Channel {
    schema_version: u32,
    skill: String,
    version: String,
    release_key: String,
    release_sha256: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Index {
    schema_version: u32,
    skill: String,
    pub(super) version: String,
    key: String,
    sha256: String,
    bytes: u64,
    pub(super) minimum_cli_version: String,
    bootstrap_schema: u32,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Record {
    schema_version: u32,
    #[serde(default = "default_skill")]
    skill: String,
    pub(super) version: String,
    files: BTreeMap<String, String>,
    #[serde(default)]
    archive_sha256: String,
}

fn invalid_skill() -> Failure {
    failure("SKILL_PACKAGE_INVALID", "Skill 发行索引、文件或摘要无效。")
}
fn changed_skill() -> Failure {
    failure(
        "SKILL_LOCAL_CHANGED",
        "Skill 本地内容已修改；请保留修改后选择独立目录。",
    )
}

fn files(root: &Path) -> Result<BTreeMap<String, String>> {
    fn visit(
        root: &Path,
        path: &Path,
        out: &mut BTreeMap<String, String>,
        total: &mut u64,
    ) -> Result<()> {
        for entry in fs::read_dir(path).map_err(|_| io_error())? {
            let entry = entry.map_err(|_| io_error())?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(|_| io_error())?;
            if metadata.is_dir() {
                visit(root, &path, out, total)?;
            } else if metadata.is_file() {
                let key = path
                    .strip_prefix(root)
                    .map_err(|_| invalid_skill())?
                    .to_str()
                    .ok_or_else(invalid_skill)?
                    .replace('\\', "/");
                if key == RECORD {
                    continue;
                }
                *total += metadata.len();
                if *total > LIMIT || out.len() >= crate::constants::distribution::MAX_SKILL_FILES {
                    return Err(invalid_skill());
                }
                out.insert(
                    key,
                    sha(&read_limited(
                        &path,
                        crate::constants::distribution::MAX_SKILL_FILE_BYTES,
                    )?),
                );
            } else {
                return Err(invalid_skill());
            }
        }
        Ok(())
    }
    plain_directory(root)?;
    let mut out = BTreeMap::new();
    visit(root, root, &mut out, &mut 0)?;
    Ok(out)
}
fn entry_version(root: &Path, name: &str) -> Result<String> {
    let text = String::from_utf8(read_limited(
        &root.join(crate::constants::distribution::SKILL_ENTRY_FILE),
        crate::constants::distribution::MAX_SKILL_FILE_BYTES,
    )?)
    .map_err(|_| invalid_skill())?;
    let header = text
        .strip_prefix("---\n")
        .and_then(|v| v.split_once("\n---\n"))
        .map(|v| v.0)
        .ok_or_else(invalid_skill)?;
    if !header.lines().any(|line| line == format!("name: {name}")) {
        return Err(invalid_skill());
    }
    let value = header
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("version:")
                .map(|v| v.trim().trim_matches('"').to_owned())
        })
        .ok_or_else(invalid_skill)?;
    version(&value)?;
    Ok(value)
}
pub(super) fn inspect(root: &Path, name: &str) -> Result<Record> {
    let mut current = Record {
        schema_version: crate::constants::protocol::FILE_SCHEMA,
        skill: name.to_owned(),
        version: entry_version(root, name)?,
        files: files(root)?,
        archive_sha256: String::new(),
    };
    if root.join(RECORD).exists() {
        let recorded: Record = serde_json::from_slice(&read_limited(
            &root.join(RECORD),
            crate::constants::distribution::INSTALL_RECORD_BYTES,
        )?)
        .map_err(|_| invalid_skill())?;
        if recorded.schema_version != crate::constants::protocol::FILE_SCHEMA
            || recorded.skill != name
            || recorded.version != current.version
            || recorded.files != current.files
        {
            return Err(changed_skill());
        }
        current.archive_sha256 = recorded.archive_sha256;
    }
    Ok(current)
}
fn save(root: &Path, record: &Record) -> Result<()> {
    atomic_write(
        &root.join(RECORD),
        &serde_json::to_vec(record).map_err(|_| invalid_skill())?,
    )
}
fn unpack(bytes: &[u8], root: &Path, name: &str, expected_version: &str) -> Result<Record> {
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|_| invalid_skill())?;
    if archive.len() > crate::constants::distribution::MAX_SKILL_FILES {
        return Err(invalid_skill());
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut total = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|_| invalid_skill())?;
        let key = entry
            .name()
            .strip_prefix(&format!("{name}/"))
            .ok_or_else(invalid_skill)?
            .to_owned();
        if key.is_empty() && entry.is_dir() {
            continue;
        }
        let trimmed = key.trim_end_matches('/');
        if !trimmed.split('/').all(|p| {
            !p.is_empty()
                && p != "."
                && p != ".."
                && !p.ends_with('.')
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                && ![
                    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6",
                    "com7", "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7",
                    "lpt8", "lpt9",
                ]
                .contains(
                    &p.split('.')
                        .next()
                        .unwrap_or("")
                        .to_ascii_lowercase()
                        .as_str(),
                )
        }) || trimmed == RECORD
            || !seen.insert(trimmed.to_ascii_lowercase())
        {
            return Err(invalid_skill());
        }
        let mode = entry
            .unix_mode()
            .unwrap_or(crate::constants::storage::PORTABLE_FILE_MODE);
        if mode & crate::constants::storage::FILE_KIND_MASK != 0
            && mode & crate::constants::storage::FILE_KIND_MASK
                != crate::constants::storage::REGULAR_FILE_KIND
            && !(entry.is_dir()
                && mode & crate::constants::storage::FILE_KIND_MASK
                    == crate::constants::storage::DIRECTORY_KIND)
        {
            return Err(invalid_skill());
        }
        let path = root.join(trimmed);
        if entry.is_dir() {
            fs::create_dir_all(&path).map_err(|_| io_error())?;
            continue;
        }
        total += entry.size();
        if entry.size() > crate::constants::distribution::MAX_SKILL_FILE_BYTES || total > LIMIT {
            return Err(invalid_skill());
        }
        fs::create_dir_all(path.parent().ok_or_else(invalid_skill)?).map_err(|_| io_error())?;
        let mut content = Vec::new();
        entry
            .by_ref()
            .take(crate::constants::distribution::MAX_SKILL_FILE_BYTES + 1)
            .read_to_end(&mut content)
            .map_err(|_| invalid_skill())?;
        if content.len() as u64 != entry.size() {
            return Err(invalid_skill());
        }
        fs::write(&path, content).map_err(|_| io_error())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                &path,
                fs::Permissions::from_mode(
                    if mode & crate::constants::storage::EXECUTABLE_MASK != 0 {
                        crate::constants::storage::EXECUTABLE_MODE
                    } else {
                        crate::constants::storage::PRIVATE_FILE_MODE
                    },
                ),
            )
            .map_err(|_| io_error())?;
        }
    }
    if entry_version(root, name)? != expected_version {
        return Err(invalid_skill());
    }
    let mut record = inspect(root, name)?;
    record.archive_sha256 = sha(bytes);
    for key in [
        "SKILL.md",
        "LICENSE",
        "scripts/distribution.json",
        "agents/openai.yaml",
    ] {
        if !record.files.contains_key(key) {
            return Err(invalid_skill());
        }
    }
    save(root, &record)?;
    Ok(record)
}

fn default_skill() -> String {
    "dt-cli".to_owned()
}

pub(super) fn skill_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > crate::constants::distribution::MAX_SKILL_NAME_LENGTH
        || name.starts_with('-')
        || name.ends_with('-')
        || name.contains("--")
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(invalid_skill());
    }
    Ok(())
}
pub(super) fn channel_key(name: &str) -> String {
    if name == "dt-cli" {
        crate::constants::distribution::LEGACY_SKILL_STABLE.into()
    } else {
        format!("channels/skills/{name}-stable.json")
    }
}
fn release_key(name: &str, version: &str) -> String {
    if name == "dt-cli" {
        format!("skills/{version}/release.json")
    } else {
        format!("skills/{name}/{version}/release.json")
    }
}
fn archive_key(name: &str, version: &str) -> String {
    if name == "dt-cli" {
        format!("skills/{version}/{name}-skill.zip")
    } else {
        format!("skills/{name}/{version}/{name}-skill.zip")
    }
}

pub(super) async fn execute(args: &clap::ArgMatches) -> Result<Value> {
    let name = string(args, "name").unwrap_or("dt-cli");
    skill_name(name)?;
    let root = PathBuf::from(string(args, "directory").ok_or_else(invalid)?);
    if !root.is_absolute()
        || root.file_name().and_then(|v| v.to_str()) != Some(name)
        || root
            .components()
            .any(|p| matches!(p, std::path::Component::ParentDir))
    {
        return Err(invalid_skill());
    }
    let parent = root.parent().ok_or_else(invalid_skill)?;
    plain_directory(parent)?;
    let _lock = lock(&root)?;
    let previous = parent.join(format!(".{name}.skill-previous"));
    let pending = parent.join(format!(".{name}.skill-pending.json"));
    if pending.exists() {
        if !root.exists() && previous.exists() {
            fs::rename(&previous, &root).map_err(|_| io_error())?;
        }
        if root.exists() {
            inspect(&root, name)?;
            fs::remove_file(&pending).map_err(|_| io_error())?;
        } else {
            // A cancelled first install has no prior tree to restore; start it again.
            fs::remove_file(&pending).map_err(|_| io_error())?;
        }
    }
    let current = if root.exists() {
        Some(inspect(&root, name)?)
    } else {
        None
    };
    let check = flag(args, "check");
    if flag(args, "rollback") {
        let old = inspect(&previous, name)?;
        if !check {
            let temporary = tempfile::tempdir_in(parent).map_err(|_| io_error())?;
            let swap = temporary.path().join("current");
            if root.exists() {
                fs::rename(&root, &swap).map_err(|_| io_error())?;
            }
            if fs::rename(&previous, &root).is_err() {
                if swap.exists() {
                    fs::rename(&swap, &root).map_err(|_| io_error())?;
                }
                return Err(io_error());
            }
            if swap.exists() {
                fs::rename(&swap, &previous).map_err(|_| io_error())?;
            }
        }
        return Ok(
            json!({"action":if check{"rollback-check"}else{"rollback"},"version":old.version,"directory":root,"changed":!check}),
        );
    }
    let source: Value = serde_json::from_str(include_str!(
        "../../skills/dt-cli/scripts/distribution.json"
    ))
    .map_err(|_| invalid_skill())?;
    let base = url::Url::parse(source["publicBaseUrl"].as_str().ok_or_else(invalid_skill)?)
        .map_err(|_| invalid_skill())?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(
            crate::constants::distribution::CONNECT_SECONDS,
        ))
        .timeout(Duration::from_secs(
            crate::constants::distribution::DOWNLOAD_SECONDS,
        ))
        .build()
        .map_err(|_| io_error())?;
    let channel: Channel =
        serde_json::from_slice(&remote::get(&client, &base, &channel_key(name), MAX_JSON).await?)
            .map_err(|_| invalid_skill())?;
    if channel.schema_version != crate::constants::protocol::FILE_SCHEMA
        || channel.skill != name
        || !digest(&channel.release_sha256)
        || channel.release_key != release_key(name, &channel.version)
    {
        return Err(invalid_skill());
    }
    version(&channel.version)?;
    let raw = remote::get(&client, &base, &channel.release_key, MAX_JSON).await?;
    if sha(&raw) != channel.release_sha256 {
        return Err(invalid_skill());
    }
    let index: Index = serde_json::from_slice(&raw).map_err(|_| invalid_skill())?;
    if index.schema_version != crate::constants::protocol::FILE_SCHEMA
        || index.skill != name
        || index.version != channel.version
        || index.key != archive_key(name, &index.version)
        || !digest(&index.sha256)
        || index.bytes == 0
        || index.bytes > LIMIT
        || index.bootstrap_schema != 1
    {
        return Err(invalid_skill());
    }
    if version(&index.minimum_cli_version)? > version(env!("CARGO_PKG_VERSION"))? {
        let mut error = failure("CLIENT_UPGRADE_REQUIRED", "技能要求更高版本的 dt-cli。");
        error.details = Some(json!({"minimumCliVersion":index.minimum_cli_version,"skill":name}));
        return Err(error);
    }
    if let Some(current) = &current
        && version(&current.version)? > version(&index.version)?
    {
        return Err(failure(
            "SKILL_DOWNGRADE_BLOCKED",
            "稳定版本低于已安装版本；需要回退时使用 --rollback。",
        ));
    }
    if current
        .as_ref()
        .is_some_and(|c| c.version == index.version && c.archive_sha256 == index.sha256)
    {
        return Ok(
            json!({"action":if check{"check"}else{"existing"},"skill":name,"version":index.version,
            "directory":root,"changed":false,"updateAvailable":false,"sha256":index.sha256,
            "bytes":index.bytes,"packageDownloaded":false,"releaseVerified":true}),
        );
    }
    let bytes = remote::get(&client, &base, &index.key, index.bytes).await?;
    if bytes.len() as u64 != index.bytes || sha(&bytes) != index.sha256 {
        return Err(invalid_skill());
    }
    let temporary = tempfile::tempdir_in(parent).map_err(|_| io_error())?;
    let staged = temporary.path().join(name);
    fs::create_dir(&staged).map_err(|_| io_error())?;
    let incoming = unpack(&bytes, &staged, name, &index.version)?;
    if let Some(current) = &current
        && current.version != index.version
        && !root.join(RECORD).exists()
    {
        // Adopt old manual imports only after matching their immutable published package.
        let old_key = release_key(name, &current.version);
        let old: Index =
            serde_json::from_slice(&remote::get(&client, &base, &old_key, MAX_JSON).await?)
                .map_err(|_| invalid_skill())?;
        if old.schema_version != crate::constants::protocol::FILE_SCHEMA
            || old.skill != name
            || old.version != current.version
            || old.key != archive_key(name, &current.version)
            || !digest(&old.sha256)
            || old.bytes == 0
            || old.bytes > LIMIT
        {
            return Err(invalid_skill());
        }
        let old_bytes = remote::get(&client, &base, &old.key, old.bytes).await?;
        if old_bytes.len() as u64 != old.bytes || sha(&old_bytes) != old.sha256 {
            return Err(invalid_skill());
        }
        let original = temporary.path().join("original");
        fs::create_dir(&original).map_err(|_| io_error())?;
        if unpack(&old_bytes, &original, name, &current.version)?.files != current.files {
            return Err(changed_skill());
        }
    }
    let changed = current.as_ref().is_none_or(|c| c.files != incoming.files);
    if current.as_ref().is_some_and(|c| c.version == index.version) && changed {
        return Err(changed_skill());
    }
    if changed && !check {
        if previous.exists() {
            inspect(&previous, name)?;
            fs::remove_dir_all(&previous).map_err(|_| io_error())?;
        }
        atomic_write(&pending, b"{\"schemaVersion\":1}")?;
        if let Some(current) = &current {
            save(&root, current)?;
            fs::rename(&root, &previous).map_err(|_| io_error())?;
        }
        if fs::rename(&staged, &root).is_err() {
            if previous.exists() {
                fs::rename(&previous, &root).map_err(|_| io_error())?;
            }
            return Err(io_error());
        }
        fs::remove_file(&pending).map_err(|_| io_error())?;
    } else if !check && !changed {
        save(&root, &incoming)?;
    }
    Ok(
        json!({"action":if check{"check"}else if !changed{"existing"}else if current.is_some(){"update"}else{"install"},
        "skill":name,"version":index.version,"directory":root,"changed":changed&&!check,"updateAvailable":changed,
        "sha256":index.sha256,"bytes":index.bytes,"releaseVerified":true,"previousDirectory":if previous.exists(){Some(previous)}else{None}}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bundle(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, data) in entries {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }
    #[test]
    fn archive_rejects_traversal_and_checks_installed_content() {
        let dir = tempfile::tempdir().unwrap();
        let invalid = bundle(&[("dt-cli/../escape", b"bad")]);
        assert!(unpack(&invalid, dir.path(), "dt-cli", "0.5.6").is_err());
        assert!(!dir.path().parent().unwrap().join("escape").exists());
        let valid = bundle(&[
            (
                "dt-cli/SKILL.md",
                b"---\nname: dt-cli\nmetadata:\n  version: \"0.5.6\"\n---\nSkill\n",
            ),
            ("dt-cli/LICENSE", b"MIT"),
            ("dt-cli/scripts/distribution.json", b"{}"),
            ("dt-cli/agents/openai.yaml", b"interface: {}"),
        ]);
        let record = unpack(&valid, dir.path(), "dt-cli", "0.5.6").unwrap();
        assert_eq!(inspect(dir.path(), "dt-cli").unwrap().files, record.files);
        fs::write(dir.path().join("LICENSE"), b"changed").unwrap();
        assert_eq!(
            inspect(dir.path(), "dt-cli").unwrap_err().code,
            "SKILL_LOCAL_CHANGED"
        );
    }
}
