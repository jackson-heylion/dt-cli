use super::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Source {
    schema_version: u32,
    public_base_url: String,
    channel_key: String,
    minimum_cli_version: String,
    skill_version: String,
    bootstrap_schema: u32,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Stable {
    schema_version: u32,
    sequence: u64,
    version: String,
    release_key: String,
    release_sha256: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Compatibility {
    bootstrap_schema: u32,
    profile_format: u32,
    credential_format: u32,
    installer_schema: u32,
    launcher_schema: u32,
    minimum_skill_version: String,
    maximum_skill_version_exclusive: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Object {
    key: String,
    sha256: String,
    bytes: u64,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Target {
    target: String,
    os: String,
    architecture: String,
    key: String,
    sha256: String,
    bytes: u64,
    binary_sha256: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Release {
    schema_version: u32,
    version: String,
    build_commit: String,
    catalog_digest: String,
    compatibility: Compatibility,
    packages: Vec<Target>,
    skills: Vec<Object>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Cache {
    checked_at: i64,
    sequence: u64,
    version: String,
    release_sha256: String,
}
fn unavailable() -> Failure {
    failure(
        "DISTRIBUTION_UNAVAILABLE",
        "无法从固定 HTTPS 分发入口取得经过校验的发行物。",
    )
}
fn index_invalid() -> Failure {
    failure(
        "DISTRIBUTION_INVALID",
        "发行索引的来源、版本、兼容范围或摘要无效。",
    )
}
fn object_key(key: &str) -> Result<()> {
    if key.len() > 200
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"./_-".contains(&b))
        || !key
            .split('/')
            .all(|p| !p.is_empty() && p != "." && p != "..")
        || !(key.starts_with("releases/") || key.starts_with("skills/"))
    {
        return Err(index_invalid());
    }
    Ok(())
}
fn source_url(source: &Source) -> Result<url::Url> {
    let url = url::Url::parse(&source.public_base_url).map_err(|_| {
        failure(
            "DISTRIBUTION_NOT_CONFIGURED",
            "发行物尚未配置固定 HTTPS 下载入口。",
        )
    })?;
    if source.schema_version != 1
        || source.bootstrap_schema != 1
        || source.channel_key != "channels/stable.json"
        || url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.path().ends_with('/')
    {
        return Err(index_invalid());
    }
    Ok(url)
}
async fn get(client: &reqwest::Client, base: &url::Url, key: &str, limit: u64) -> Result<Vec<u8>> {
    let url = base.join(key).map_err(|_| index_invalid())?;
    if url.origin() != base.origin() || !url.path().starts_with(base.path()) {
        return Err(index_invalid());
    }
    let mut response = client
        .get(url)
        .header("Cache-Control", "no-cache")
        .send()
        .await
        .map_err(|_| unavailable())?;
    if !response.status().is_success() || response.content_length().is_some_and(|n| n > limit) {
        return Err(unavailable());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| unavailable())? {
        if bytes.len() as u64 + chunk.len() as u64 > limit {
            return Err(index_invalid());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
fn validate<'a>(stable: &Stable, release: &'a Release, source: &Source) -> Result<&'a Target> {
    let compatible = &release.compatibility;
    if stable.schema_version != 1
        || stable.sequence == 0
        || release.schema_version != 1
        || stable.version != release.version
        || version(&release.version)? < version(&source.minimum_cli_version)?
        || stable.release_key != format!("releases/{}/release.json", stable.version)
        || release.build_commit.len() != 40
        || !release
            .build_commit
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        || !digest(&release.catalog_digest)
        || compatible.bootstrap_schema != 1
        || compatible.profile_format != 1
        || compatible.credential_format != 1
        || compatible.installer_schema != 1
        || compatible.launcher_schema != 1
        || version(&source.skill_version)? < version(&compatible.minimum_skill_version)?
        || version(&source.skill_version)? >= version(&compatible.maximum_skill_version_exclusive)?
    {
        return Err(index_invalid());
    }
    let matrix: Value =
        serde_json::from_str(include_str!("../../catalog/release-targets.json")).unwrap();
    let expected = matrix["targets"].as_array().unwrap();
    if release.packages.len() != expected.len() {
        return Err(index_invalid());
    }
    for item in expected {
        let targets: Vec<_> = release
            .packages
            .iter()
            .filter(|p| p.target == item["target"])
            .collect();
        if targets.len() != 1 {
            return Err(index_invalid());
        }
        let target = targets[0];
        object_key(&target.key)?;
        if target.os != item["os"]
            || target.architecture != item["architecture"]
            || !target
                .key
                .starts_with(&format!("releases/{}/", release.version))
            || !digest(&target.sha256)
            || !digest(&target.binary_sha256)
            || target.bytes == 0
            || target.bytes > MAX_BINARY
        {
            return Err(index_invalid());
        }
    }
    for object in &release.skills {
        object_key(&object.key)?;
        if !digest(&object.sha256) || object.bytes == 0 || object.bytes > MAX_JSON * 64 {
            return Err(index_invalid());
        }
    }
    release
        .packages
        .iter()
        .find(|p| (p.os.as_str(), p.architecture.as_str()) == platform())
        .ok_or_else(|| failure("PLATFORM_MISMATCH", "分发入口没有适用于当前平台的原生包。"))
}
pub(super) async fn upgrade(args: &clap::ArgMatches) -> Result<Value> {
    if flag(args, "rollback")
        || flag(args, "prune")
        || flag(args, "candidate")
        || string(args, "package").is_some()
        || string(args, "sha256").is_some()
        || string(args, "signer").is_some()
    {
        return Err(invalid());
    }
    let source: Source = serde_json::from_str(include_str!(
        "../../skills/dt-cli/scripts/distribution.json"
    ))
    .unwrap();
    let root = string(args, "directory")
        .map(PathBuf::from)
        .unwrap_or(running_root().unwrap_or(default_root(false)?));
    let root = if root.is_absolute() {
        root
    } else {
        std::env::current_dir().map_err(|_| io_error())?.join(root)
    };
    version(string(args, "minimum-version").unwrap_or(&source.minimum_cli_version))?;
    let selected = Selection::read(&root)?;
    let (current, _) = manifest_at(&root, &selected.current, platform())?;
    let minimum = string(args, "minimum-version").unwrap_or(&source.minimum_cli_version);
    let cache_path = root.join("distribution-cache.json");
    let cache = read_limited(&cache_path, MAX_JSON)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Cache>(&bytes).ok());
    let now = chrono::Utc::now().timestamp();
    let base = source_url(&source)?;
    if flag(args, "cached")
        && version(&current.version)? >= version(minimum)?
        && cache.as_ref().is_some_and(|c| {
            now >= c.checked_at && now - c.checked_at < 86400 && c.version == current.version
        })
    {
        return Ok(
            json!({"action":"upgrade-check","version":current.version,"launcher":root.join("bin").join(binary_name(platform().0)),"changed":false,"updateCheck":"cached","httpRequests":0}),
        );
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(90))
        .build()
        .map_err(|_| unavailable())?;
    let work = async {
        let stable: Stable =
            serde_json::from_slice(&get(&client, &base, &source.channel_key, MAX_JSON).await?)
                .map_err(|_| index_invalid())?;
        object_key(&stable.release_key)?;
        if !digest(&stable.release_sha256) {
            return Err(index_invalid());
        }
        if cache.as_ref().is_some_and(|c| {
            stable.sequence < c.sequence
                || version(&stable.version).ok() < version(&c.version).ok()
                || stable.sequence == c.sequence
                    && (stable.release_sha256 != c.release_sha256 || stable.version != c.version)
        }) {
            return Err(failure(
                "DISTRIBUTION_ROLLBACK_BLOCKED",
                "发行索引倒退或同一发布序号的内容发生变化。",
            ));
        }
        let bytes = get(&client, &base, &stable.release_key, MAX_JSON * 4).await?;
        if sha(&bytes) != stable.release_sha256 {
            return Err(index_invalid());
        }
        let release: Release = serde_json::from_slice(&bytes).map_err(|_| index_invalid())?;
        let target = validate(&stable, &release, &source)?;
        if version(&release.version)? < version(minimum)? {
            return Err(failure(
                "CLIENT_UPGRADE_REQUIRED",
                "stable 发行版本尚未满足本次任务的最低版本要求。",
            ));
        }
        let available = version(&release.version)? > version(&current.version)?;
        if version(&release.version)? == version(&current.version)?
            && target.binary_sha256 != current.sha256
        {
            return Err(index_invalid());
        }
        let mut result = json!({"action":"upgrade-check","version":current.version,"latestVersion":release.version,"updateAvailable":available,"changed":false,"updateCheck":"online","httpRequests":2});
        if available && !flag(args, "check") {
            let data = get(&client, &base, &target.key, target.bytes).await?;
            if data.len() as u64 != target.bytes || sha(&data) != target.sha256 {
                return Err(bad_package());
            }
            let temporary = tempfile::tempdir().map_err(|_| io_error())?;
            let archive = temporary.path().join("release.zip");
            fs::write(&archive, data).map_err(|_| io_error())?;
            let package = Package::read(&archive, Some(&target.sha256), platform())?;
            if package.manifest.version != release.version
                || package.manifest.build_commit != release.build_commit
                || package.manifest.catalog_digest != release.catalog_digest
                || package.manifest.sha256 != target.binary_sha256
            {
                return Err(index_invalid());
            }
            result = apply(
                &root,
                Some(package),
                &Options {
                    install: false,
                    check: false,
                    rollback: false,
                    prune: false,
                    candidate: false,
                    expected: Some(&target.sha256),
                    signer: None,
                },
                platform(),
                &SystemVerifier,
            )?;
            result["updateCheck"] = json!("online");
            result["httpRequests"] = json!(3);
        }
        if !flag(args, "check") {
            // Cache failure does not hide a successfully completed program switch.
            let cached = Cache {
                checked_at: now,
                sequence: stable.sequence,
                version: stable.version,
                release_sha256: stable.release_sha256,
            };
            result["cacheSaved"] =
                json!(atomic_write(&cache_path, &serde_json::to_vec(&cached).unwrap()).is_ok());
        }
        result["launcher"] = json!(root.join("bin").join(binary_name(platform().0)));
        Ok(result)
    };
    tokio::time::timeout(Duration::from_secs(120), work)
        .await
        .map_err(|_| unavailable())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn distribution_keys_cannot_escape_the_fixed_prefix() {
        for value in [
            "https://evil.test/a",
            "//evil.test/a",
            "releases/../a",
            "releases/%2e%2e/a",
            "releases/a?x=1",
            "releases//a",
        ] {
            assert!(object_key(value).is_err());
        }
        assert!(object_key("releases/0.4.1/dt-cli-macos-arm64.zip").is_ok());
    }

    #[test]
    fn release_validation_rejects_missing_targets_formats_and_skill_incompatibility() {
        let source: Source = serde_json::from_value(json!({"schemaVersion":1,"publicBaseUrl":"https://download.example.test/dt-cli/",
            "channelKey":"channels/stable.json","minimumCliVersion":"0.4.1","skillVersion":"0.4.1","bootstrapSchema":1})).unwrap();
        let stable: Stable =
            serde_json::from_value(json!({"schemaVersion":1,"sequence":1,"version":"0.4.1",
            "releaseKey":"releases/0.4.1/release.json","releaseSha256":"a".repeat(64)}))
            .unwrap();
        let value = json!({"schemaVersion":1,"version":"0.4.1","buildCommit":"a".repeat(40),"catalogDigest":"b".repeat(64),
            "compatibility":{"bootstrapSchema":1,"profileFormat":1,"credentialFormat":1,"installerSchema":1,"launcherSchema":1,
                "minimumSkillVersion":"0.4.1","maximumSkillVersionExclusive":"0.5.0"},
            "packages":[
                {"target":"aarch64-apple-darwin","os":"Darwin","architecture":"arm64","key":"releases/0.4.1/mac.zip","sha256":"c".repeat(64),"bytes":10,"binarySha256":"d".repeat(64)},
                {"target":"x86_64-pc-windows-msvc","os":"Windows","architecture":"x86_64","key":"releases/0.4.1/win.zip","sha256":"e".repeat(64),"bytes":10,"binarySha256":"f".repeat(64)}],"skills":[]});
        for path in [
            "missing",
            "profile",
            "skill",
            "target",
            "path",
            "oversized",
            "dirty",
        ] {
            let mut changed = value.clone();
            match path {
                "missing" => {
                    changed["packages"].as_array_mut().unwrap().pop();
                }
                "profile" => changed["compatibility"]["profileFormat"] = json!(2),
                "skill" => changed["compatibility"]["minimumSkillVersion"] = json!("0.5.0"),
                "target" => changed["packages"][0]["architecture"] = json!("x86_64"),
                "path" => changed["packages"][0]["key"] = json!("releases/../escape.zip"),
                "oversized" => changed["packages"][0]["bytes"] = json!(MAX_BINARY + 1),
                "dirty" => changed["buildCommit"] = json!(format!("{}+dirty", "a".repeat(40))),
                _ => unreachable!(),
            }
            let release: Release = serde_json::from_value(changed).unwrap();
            assert_eq!(
                validate(&stable, &release, &source).unwrap_err().code,
                "DISTRIBUTION_INVALID",
                "{path}"
            );
        }
    }
}
