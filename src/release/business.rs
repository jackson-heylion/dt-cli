//! New business tasks use verified stable content; their execution resources remain immutable.
use super::*;

const RUN_SCHEMA: u32 = crate::constants::protocol::FILE_SCHEMA;
const RUN_DIRECTORY: &str = crate::constants::distribution::RUN_DIRECTORY;
const SNAPSHOT_DIRECTORY: &str = crate::constants::distribution::SNAPSHOT_DIRECTORY;
const RUN_RECORD: &str = crate::constants::distribution::RUN_RECORD;
const REQUIREMENTS: &str = crate::constants::distribution::REQUIREMENTS_FILE;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RunRecord {
    schema_version: u32,
    run_id: String,
    skill: String,
    skill_version: String,
    cli_version: String,
    created_at: String,
    profile: String,
    permission: crate::governed::SkillPermission,
    package: skill::Record,
    resources_directory: PathBuf,
    state: String,
    client_read_confirmed: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreparedRun {
    run_id: String,
    run_directory: PathBuf,
    resources_directory: PathBuf,
    entry_file: PathBuf,
    skill: String,
    skill_version: String,
    cli_executable: PathBuf,
    permission_verified: bool,
    client_read_confirmed: bool,
    next_action: String,
}
#[derive(Deserialize)]
struct ProcessEnvelope<T> {
    ok: bool,
    data: Option<T>,
}

pub(super) async fn prepare(rt: &crate::Runtime, args: &clap::ArgMatches) -> Result<Value> {
    let name = string(args, "name").ok_or_else(invalid)?;
    skill::skill_name(name)?;
    let profile = string(args, "profile").ok_or_else(invalid)?;
    match skill::execute(args).await {
        Ok(_) => {}
        Err(error) if error.code == "CLIENT_UPGRADE_REQUIRED" => {
            let minimum = error
                .details
                .as_ref()
                .and_then(|v| v["minimumCliVersion"].as_str())
                .ok_or_else(invalid)?;
            let command = crate::catalog::Catalog::shared().command();
            let parsed = command
                .try_get_matches_from([
                    "dt-cli",
                    "upgrade",
                    "--online",
                    "--minimum-version",
                    minimum,
                ])
                .map_err(|_| invalid())?;
            let (_, leaf) = crate::catalog::Catalog::shared().resolve(&parsed)?;
            let updated = remote::upgrade(leaf).await?;
            let launcher = PathBuf::from(updated["launcher"].as_str().ok_or_else(io_error)?);
            let directory = string(args, "directory").ok_or_else(invalid)?;
            let output = verifier::run_bounded(
                Command::new(&launcher).args([
                    "skill",
                    "prepare",
                    "--name",
                    name,
                    "--directory",
                    directory,
                    "--profile",
                    profile,
                ]),
                Duration::from_secs(crate::constants::distribution::PREPARE_SECONDS),
                crate::constants::distribution::RUN_RECORD_BYTES,
            )?;
            let envelope: ProcessEnvelope<PreparedRun> =
                serde_json::from_slice(&output).map_err(|_| io_error())?;
            if !envelope.ok {
                return Err(failure(
                    "SKILL_PREPARE_FAILED",
                    "工具已更新，但技能准备未完成。",
                ));
            }
            return serde_json::to_value(envelope.data.ok_or_else(io_error)?)
                .map_err(|_| io_error());
        }
        Err(error) => return Err(error),
    }
    let source = PathBuf::from(string(args, "directory").ok_or_else(invalid)?);
    let _lock = lock(&source)?;
    let installed = skill::inspect(&source, name)?;
    let requirements: crate::governed::SkillRequirements = serde_json::from_slice(&read_limited(
        &source.join(REQUIREMENTS),
        crate::constants::distribution::REQUIREMENTS_BYTES,
    )?)
    .map_err(|_| bad_package())?;
    let permission = crate::governed::prepare_skill(rt, profile, &requirements).await?;
    let id = crate::login::random();
    let root = rt.root.join(RUN_DIRECTORY);
    crate::private_store::directory(&root)?;
    let path = root.join(&id);
    crate::private_store::directory(&path)?;
    let resources = path.join(SNAPSHOT_DIRECTORY);
    copy_resources(&source, &resources)?;
    if installed != skill::inspect(&resources, name)? {
        return Err(failure(
            "SKILL_LOCAL_CHANGED",
            "复制运行资料时技能内容发生变化。",
        ));
    }
    let prepared = PreparedRun {
        run_id: id.clone(),
        run_directory: path.clone(),
        resources_directory: resources.clone(),
        entry_file: resources.join(crate::constants::distribution::SKILL_ENTRY_FILE),
        skill: name.to_owned(),
        skill_version: installed.version.clone(),
        cli_executable: std::env::current_exe().map_err(|_| io_error())?,
        permission_verified: true,
        client_read_confirmed: false,
        next_action: "read-snapshot-entry-before-analysis".to_owned(),
    };
    let record = RunRecord {
        schema_version: RUN_SCHEMA,
        run_id: id,
        skill: name.to_owned(),
        skill_version: installed.version.clone(),
        cli_version: env!("CARGO_PKG_VERSION").to_owned(),
        created_at: chrono::Utc::now().to_rfc3339(),
        profile: profile.to_owned(),
        permission,
        package: installed,
        resources_directory: resources,
        state: "prepared".to_owned(),
        client_read_confirmed: false,
    };
    crate::private_store::write_metadata(
        &path.join(RUN_RECORD),
        &record,
        crate::constants::distribution::RUN_RECORD_BYTES,
    )?;
    serde_json::to_value(prepared).map_err(|_| io_error())
}

fn copy_resources(source: &Path, target: &Path) -> Result<()> {
    crate::private_store::directory(target)?;
    for entry in fs::read_dir(source).map_err(|_| io_error())? {
        let entry = entry.map_err(|_| io_error())?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(|_| io_error())?;
        let destination = target.join(entry.file_name());
        if metadata.is_dir() {
            copy_resources(&entry.path(), &destination)?;
        } else if metadata.is_file() {
            atomic_write(
                &destination,
                &read_limited(
                    &entry.path(),
                    crate::constants::distribution::MAX_SKILL_FILE_BYTES,
                )?,
            )?;
        } else {
            return Err(bad_package());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn task_resources_survive_the_next_skill_change() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("calculation.py"), b"old rule").unwrap();
        let snapshot = root.path().join("snapshot");
        copy_resources(&source, &snapshot).unwrap();
        fs::write(source.join("calculation.py"), b"new rule").unwrap();
        assert_eq!(
            fs::read(snapshot.join("calculation.py")).unwrap(),
            b"old rule"
        );
    }
}
