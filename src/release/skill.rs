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

mod archive;
use archive::{inspect, save, unpack};

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
        crate::constants::distribution::UNIVERSAL_SKILL_STABLE.into()
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

pub(super) async fn execute(args: &crate::interface::request::Skill) -> Result<Value> {
    let name = args.name.as_str();
    skill_name(name)?;
    let root = PathBuf::from(args.directory.as_str());
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
    let check = args.check;
    if args.rollback {
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
        error.details = Some(Box::new(crate::output::Details {
            minimum_cli_version: Some(index.minimum_cli_version.clone()),
            skill: Some(name.into()),
            ..Default::default()
        }));
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
