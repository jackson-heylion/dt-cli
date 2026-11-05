//! 聚合诊断复用固定 HTTPS 分发源、摘要、兼容范围和防倒退验证。
use super::*;
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StableDiagnostic {
    /// 从固定可信入口核验的正式版本。
    pub version: String,
    /// 防倒退发布序号。
    pub sequence: u64,
    /// 已核验的本机目标原生包摘要。
    pub binary_sha256: String,
    /// 本次检查的接口版本下限。
    pub minimum_cli_version: String,
    /// 正式版是否满足接口最低版本。
    pub minimum_satisfied: bool,
}
pub(crate) async fn stable_diagnostic(minimum: &str) -> Result<StableDiagnostic> {
    let source: Source = serde_json::from_str(include_str!(
        "../../skills/dt-cli/scripts/distribution.json"
    ))
    .map_err(|_| index_invalid())?;
    let base = source_url(&source)?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
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
        let root = running_root().unwrap_or(default_root(false)?);
        let cache = read_limited(
            &root.join(crate::constants::distribution::NATIVE_CACHE_FILE),
            MAX_JSON,
        )
        .ok()
        .and_then(|b| serde_json::from_slice::<Cache>(&b).ok());
        if cache
            .as_ref()
            .is_some_and(|c| c.blocks(&stable, &source.channel_key))
        {
            return Err(index_invalid());
        }
        let bytes = get(&client, &base, &stable.release_key, MAX_JSON * 4).await?;
        if sha(&bytes) != stable.release_sha256 {
            return Err(index_invalid());
        }
        let release: Release = serde_json::from_slice(&bytes).map_err(|_| index_invalid())?;
        let target = validate(&stable, &release, &source)?;
        Ok(StableDiagnostic {
            version: release.version.clone(),
            sequence: stable.sequence,
            binary_sha256: target.binary_sha256.clone(),
            minimum_cli_version: minimum.into(),
            minimum_satisfied: version(&release.version)? >= version(minimum)?,
        })
    };
    tokio::time::timeout(Duration::from_secs(12), work)
        .await
        .map_err(|_| unavailable())?
}
