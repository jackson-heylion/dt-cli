use crate::output::{Failure, Result, invalid, storage};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Profile {
    pub environment: String,
    pub subject_id: String,
    pub authorization_id: String,
    pub authorization_expires_at: String,
    pub access_expires_at: String,
    pub checked_at: String,
    pub credential_key: String,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Environment {
    pub api_origin: String,
    pub portal_origin: String,
    pub recovery_url: String,
    /// External path of the fixed IAM launch entry, including any gateway base path.
    pub launch_path: String,
}
impl Environment {
    pub fn validate(&self) -> Result<()> {
        let api = url::Url::parse(&self.api_origin).map_err(|_| invalid())?;
        if api.scheme() != "https"
            || api.host_str().is_none()
            || api.path().ends_with('/')
            || api.query().is_some()
            || api.fragment().is_some()
            || !api.username().is_empty()
            || api.password().is_some()
        {
            return Err(invalid());
        }
        let portal = url::Url::parse(&self.portal_origin).map_err(|_| invalid())?;
        if self.portal_origin != portal.origin().ascii_serialization()
            || portal.scheme() != "https"
            || portal.host_str().is_none()
            || portal.query().is_some()
            || portal.fragment().is_some()
            || !portal.username().is_empty()
            || portal.password().is_some()
        {
            return Err(invalid());
        }
        let recovery = url::Url::parse(&self.recovery_url).map_err(|_| invalid())?;
        if recovery.origin() != portal.origin()
            || !recovery.username().is_empty()
            || recovery.password().is_some()
        {
            return Err(invalid());
        }
        if !self.launch_path.starts_with('/')
            || self.launch_path.ends_with('/')
            || self.launch_path.contains("//")
            || self.launch_path.contains(['?', '#', '\\'])
            || self.launch_path.len() > 200
        {
            return Err(invalid());
        }
        Ok(())
    }
}
pub fn environments() -> Result<BTreeMap<String, Environment>> {
    // Only release-owned input; neither cwd nor environment variables can override origins.
    #[allow(unused_mut)]
    let mut envs: BTreeMap<String, Environment> =
        serde_json::from_str(include_str!("../catalog/environments.json"))
            .map_err(|_| invalid())?;
    #[cfg(feature = "local-dev")]
    envs.insert(
        "local".into(),
        Environment {
            api_origin: "http://localhost:10998".into(),
            portal_origin: "http://localhost:10998".into(),
            recovery_url: "http://localhost:10998/cli-authorizations.html".into(),
            launch_path: "/cli/launch".into(),
        },
    );
    for (name, env) in &envs {
        #[cfg(feature = "local-dev")]
        if name == "local" {
            env.validate_local()?;
            continue;
        }
        let _ = name;
        env.validate()?;
    }
    Ok(envs)
}

#[cfg(feature = "local-dev")]
impl Environment {
    fn validate_local(&self) -> Result<()> {
        if self.api_origin != "http://localhost:10998"
            || self.portal_origin != "http://localhost:10998"
            || self.recovery_url != "http://localhost:10998/cli-authorizations.html"
            || self.launch_path != "/cli/launch"
        {
            return Err(invalid());
        }
        Ok(())
    }
}
pub fn config_dir() -> Result<PathBuf> {
    directories::ProjectDirs::from("com", "datousoft", "dt-cli")
        .map(|p| p.config_dir().to_owned())
        .ok_or_else(storage)
}
pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 80
        || name.chars().any(|c| c.is_control() || "/\\:".contains(c))
    {
        Err(invalid())
    } else {
        Ok(())
    }
}
pub fn read(root: &Path, name: &str) -> Result<Option<Profile>> {
    validate_name(name)?;
    match fs::read(root.join(format!("{name}.json"))) {
        Ok(b) => {
            let invalid = || {
                Failure::new(
                    "PROFILE_INVALID",
                    2,
                    "profile 配置或凭证绑定不一致；未读取凭证。",
                )
            };
            let profile: Profile = serde_json::from_slice(&b).map_err(|_| invalid())?;
            let expected = format!(
                "{}/{}/{}",
                profile.environment, profile.subject_id, profile.authorization_id
            );
            if profile.credential_key != expected {
                return Err(invalid());
            }
            Ok(Some(profile))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(crate::private_store::context(
            crate::private_store::io_failure("read", e),
            "profile",
        )),
    }
}
pub fn save(root: &Path, name: &str, p: &Profile) -> Result<()> {
    validate_name(name)?;
    crate::private_store::write_metadata(&root.join(format!("{name}.json")), p, 16 * 1024)
        .map_err(|e| crate::private_store::context(e, "profile"))
}

pub struct Lock(File);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}
pub async fn lock(root: &Path, name: &str) -> Result<Lock> {
    validate_name(name)?;
    fs::create_dir_all(root).map_err(|e| {
        crate::private_store::context(
            crate::private_store::io_failure("create_directory", e),
            "profile_lock",
        )
    })?;
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join(format!("{name}.lock")))
        .map_err(|e| {
            crate::private_store::context(
                crate::private_store::io_failure("open_lock", e),
                "profile_lock",
            )
        })?;
    let start = Instant::now();
    loop {
        match f.try_lock_exclusive() {
            Ok(()) => return Ok(Lock(f)),
            Err(e) if e.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {}
            Err(e) => {
                return Err(crate::private_store::context(
                    crate::private_store::io_failure("lock", e),
                    "profile_lock",
                ));
            }
        }
        if start.elapsed() >= Duration::from_secs(10) {
            return Err(Failure::new("TIMEOUT", 5, "等待 profile 锁超时。"));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
pub async fn authorization_lock(root: &Path, authorization_id: &str) -> Result<Lock> {
    use sha2::{Digest, Sha256};
    let key = format!("auth-{:x}", Sha256::digest(authorization_id.as_bytes()));
    lock(root, &key).await
}

#[cfg(test)]
mod tests {
    use super::Environment;

    fn environment(api_origin: &str, portal_origin: &str) -> Environment {
        Environment {
            api_origin: api_origin.into(),
            portal_origin: portal_origin.into(),
            recovery_url: format!("{portal_origin}/cli-consent.html"),
            launch_path: "/cli/launch".into(),
        }
    }

    #[test]
    fn launch_path_must_be_a_clean_absolute_path() {
        for path in [
            "",
            "cli/launch",
            "/cli/launch/",
            "/cli//launch",
            "/cli/launch?a=b",
        ] {
            let mut environment = environment(
                "https://iam.datousoft.com/dt/iam",
                "https://portal.datousoft.com",
            );
            environment.launch_path = path.into();
            assert!(environment.validate().is_err(), "{path}");
        }
        environment(
            "https://iam.datousoft.com/dt/iam",
            "https://portal.datousoft.com",
        )
        .validate()
        .unwrap();
    }

    #[test]
    fn accepts_a_gateway_api_base_path_and_a_strict_portal_origin() {
        environment(
            "https://gateway-stg.datousoft.com/dt/iam",
            "https://gateway-stg.datousoft.com",
        )
        .validate()
        .unwrap();

        assert!(
            environment(
                "https://gateway-stg.datousoft.com/dt/iam/",
                "https://gateway-stg.datousoft.com",
            )
            .validate()
            .is_err()
        );
        assert!(
            environment(
                "https://gateway-stg.datousoft.com/dt/iam",
                "https://gateway-stg.datousoft.com/",
            )
            .validate()
            .is_err()
        );
        assert!(
            environment("http://localhost:10998", "http://localhost:10998")
                .validate()
                .is_err()
        );
    }

    #[cfg(feature = "local-dev")]
    #[test]
    fn local_debug_origin_is_fixed_and_separate_from_release_validation() {
        let environments = super::environments().unwrap();
        let local = environments.get("local").unwrap();
        assert_eq!(local.api_origin, "http://localhost:10998");
        assert!(local.validate_local().is_ok());
        let mut changed = local.clone();
        changed.api_origin = "http://localhost:10999".into();
        assert!(changed.validate_local().is_err());
        assert!(local.validate().is_err());
    }
}
