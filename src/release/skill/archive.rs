//! Skill 安全文件核验与归档解包。
use super::*;
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
pub(super) fn save(root: &Path, record: &Record) -> Result<()> {
    atomic_write(
        &root.join(RECORD),
        &serde_json::to_vec(record).map_err(|_| invalid_skill())?,
    )
}
pub(super) fn unpack(
    bytes: &[u8],
    root: &Path,
    name: &str,
    expected_version: &str,
) -> Result<Record> {
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
