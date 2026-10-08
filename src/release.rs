//! Verified installation and upgrades. Business credentials never enter this module.
use crate::{
    catalog::{flag, string},
    output::{Failure, Result, invalid},
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const MAX_BINARY: u64 = 256 * 1024 * 1024;
const MAX_JSON: u64 = 16 * 1024;
const SCHEMA: u32 = 1;
const MARKER: &str = "dt-cli.launcher.json";

pub fn build_info() -> Value {
    serde_json::from_str(env!("BUILD_METADATA")).expect("build metadata")
}
fn failure(code: &'static str, message: &'static str) -> Failure {
    let mut error = Failure::new(code, 1, message);
    error.hint = "使用 dt-cli upgrade --check 查看安装；从组织可信入口获取程序包。必要时执行 dt-cli upgrade --rollback。";
    error.retryable = false;
    error
}
fn io_error() -> Failure {
    failure(
        "INSTALLATION_IO",
        "无法访问程序安装目录；原有 profile 和凭证保持原位。",
    )
}
fn bad_package() -> Failure {
    failure(
        "PACKAGE_INVALID",
        "程序包的结构、版本、平台或摘要不符合安装合同。",
    )
}
fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn version(value: &str) -> Result<(u64, u64, u64)> {
    let parts: Vec<_> = value.split('.').collect();
    if parts.len() != 3
        || parts.iter().any(|p| {
            p.is_empty()
                || !p.bytes().all(|c| c.is_ascii_digit())
                || (p.len() > 1 && p.starts_with('0'))
        })
    {
        return Err(bad_package());
    }
    Ok((
        parts[0].parse().map_err(|_| bad_package())?,
        parts[1].parse().map_err(|_| bad_package())?,
        parts[2].parse().map_err(|_| bad_package())?,
    ))
}
fn platform() -> (&'static str, &'static str) {
    (
        if cfg!(target_os = "macos") {
            "Darwin"
        } else if cfg!(target_os = "windows") {
            "Windows"
        } else {
            "unsupported"
        },
        if cfg!(target_arch = "aarch64") {
            "arm64"
        } else if cfg!(target_arch = "x86_64") {
            "x86_64"
        } else {
            "unsupported"
        },
    )
}
fn binary_name(os: &str) -> &'static str {
    if os == "Windows" {
        "dt-cli.exe"
    } else {
        "dt-cli"
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    manifest_schema_version: u32,
    release_type: String,
    version: String,
    os: String,
    architecture: String,
    binary: String,
    sha256: String,
    build_commit: String,
    build_target: String,
    local_development: bool,
    catalog_version: String,
    catalog_digest: String,
    source_version: String,
    profile_format: u32,
    credential_format: u32,
    minimum_installer_schema: u32,
    minimum_launcher_schema: u32,
    native_probe: String,
}
impl Manifest {
    fn validate(&self, host: (&str, &str)) -> Result<()> {
        let matrix: Value = serde_json::from_str(include_str!("../catalog/release-targets.json"))
            .expect("release targets");
        let target = matrix["targets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["target"] == self.build_target)
            .ok_or_else(bad_package)?;
        if target["os"] != self.os
            || target["architecture"] != self.architecture
            || target["binary"] != self.binary
        {
            return Err(bad_package());
        }
        if (self.os.as_str(), self.architecture.as_str()) != host {
            return Err(failure(
                "PLATFORM_MISMATCH",
                "程序包与当前操作系统或架构不匹配。",
            ));
        }
        if self.manifest_schema_version != SCHEMA
            || self.minimum_installer_schema > SCHEMA
            || self.minimum_launcher_schema > SCHEMA
            || self.minimum_installer_schema == 0
            || self.minimum_launcher_schema == 0
        {
            return Err(failure(
                "INSTALLER_UPGRADE_REQUIRED",
                "程序包需要更新的安装或启动合同；当前安装未改变。",
            ));
        }
        if self.profile_format != 1 || self.credential_format != 1 {
            return Err(failure(
                "INSTALLATION_FORMAT_UNSUPPORTED",
                "程序包的 profile 或凭证格式需要单独迁移；不能直接升级或回退。",
            ));
        }
        if version(&self.version)? < (0, 3, 0)
            || !digest(&self.sha256)
            || !digest(&self.catalog_digest)
            || self.binary != binary_name(&self.os)
            || self.local_development
            || self.catalog_version.is_empty()
            || self.source_version.is_empty()
            || self.source_version.len() > 200
        {
            return Err(bad_package());
        }
        let clean = self.build_commit.len() == 40
            && self
                .build_commit
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase());
        let dirty = self.build_commit.strip_suffix("+dirty").is_some_and(|s| {
            s.len() == 40
                && s.bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        });
        match self.release_type.as_str() {
            "release" if clean && self.native_probe == "performed" => (),
            "candidate" if (clean || dirty) && self.native_probe == "not-performed" => (),
            _ => return Err(bad_package()),
        }
        Ok(())
    }
    fn matches_probe(&self, value: &Value) -> Result<()> {
        let info = &value["data"];
        if value["ok"] != true
            || info["cliVersion"] != self.version
            || info["buildCommit"] != self.build_commit
            || info["buildTarget"] != self.build_target
            || info["localDevelopment"] != false
            || info["catalogVersion"] != self.catalog_version
            || info["catalogDigest"] != self.catalog_digest
            || info["sourceVersion"] != self.source_version
        {
            return Err(failure(
                "PACKAGE_PROVENANCE_MISMATCH",
                "程序实际版本或构建来源与清单不一致；当前安装未改变。",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Identity {
    signer: String,
    identifier: Option<String>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Trust {
    schema_version: u32,
    release_type: String,
    identity: Option<Identity>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Launcher {
    schema_version: u32,
}
struct Selection {
    current: String,
    previous: Option<String>,
}
impl Selection {
    fn read(root: &Path) -> Result<Self> {
        let text = String::from_utf8(read_limited(&root.join("active"), MAX_JSON)?)
            .map_err(|_| io_error())?;
        let rows: Vec<_> = text.lines().collect();
        if rows.len() != 2 || !digest(rows[0]) || (!rows[1].is_empty() && !digest(rows[1])) {
            return Err(failure(
                "INSTALLATION_INVALID",
                "当前版本记录损坏；可从可信程序包重新安装到独立目录。",
            ));
        }
        Ok(Self {
            current: rows[0].into(),
            previous: if rows[1].is_empty() {
                None
            } else {
                Some(rows[1].into())
            },
        })
    }
    fn write(&self, root: &Path) -> Result<()> {
        atomic_write(
            &root.join("active"),
            format!(
                "{}\n{}\n",
                self.current,
                self.previous.as_deref().unwrap_or("")
            )
            .as_bytes(),
        )
    }
}

mod filesystem;
use filesystem::*;
mod verifier;
use verifier::*;
mod package;
use package::*;
mod installation;
use installation::*;
mod apply;
use apply::*;
mod remote;

pub async fn execute(operation: &str, args: &clap::ArgMatches) -> Result<Value> {
    if operation == "upgrade" && flag(args, "online") {
        remote::upgrade(args).await
    } else {
        if flag(args, "cached") || string(args, "minimum-version").is_some() {
            return Err(invalid());
        }
        dispatch(operation, args)
    }
}

pub fn dispatch(operation: &str, args: &clap::ArgMatches) -> Result<Value> {
    let options = Options {
        install: operation == "install",
        check: flag(args, "check"),
        rollback: flag(args, "rollback"),
        prune: flag(args, "prune"),
        candidate: flag(args, "candidate"),
        expected: string(args, "sha256"),
        signer: string(args, "signer"),
    };
    if options.rollback && string(args, "package").is_some() || options.install && options.check {
        return Err(invalid());
    }
    let root = match string(args, "directory") {
        Some(path) => PathBuf::from(path),
        None => {
            if options.install {
                default_root(options.candidate)?
            } else {
                running_root().unwrap_or(default_root(options.candidate)?)
            }
        }
    };
    let root = if root.is_absolute() {
        root
    } else {
        std::env::current_dir().map_err(|_| io_error())?.join(root)
    };
    let package = string(args, "package")
        .map(|path| Package::read(Path::new(path), options.expected, platform()))
        .transpose()?;
    if !options.check && !options.rollback && !options.prune && package.is_none() {
        return Err(invalid());
    }
    apply(&root, package, &options, platform(), &SystemVerifier)
}
/// The same native program provides a stable launcher, without shell argument rewriting.
pub fn launch_managed() -> Result<()> {
    let exe = std::env::current_exe().map_err(|_| io_error())?;
    let parent = exe.parent().ok_or_else(io_error)?;
    if !parent.join(MARKER).exists() {
        return Ok(());
    }
    let marker: Launcher = serde_json::from_slice(&read_limited(&parent.join(MARKER), MAX_JSON)?)
        .map_err(|_| io_error())?;
    if marker.schema_version != SCHEMA {
        return Err(failure("INSTALLATION_INVALID", "启动入口版本不受支持。"));
    }
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    // Keep recovery available even when the selected binary cannot start or no longer has an upgrade command.
    let mut command_args = args.iter();
    let operation = loop {
        match command_args.next() {
            Some(arg) if arg == "--format" => {
                let _ = command_args.next();
            }
            Some(arg) if arg.to_string_lossy().starts_with("--format=") => (),
            other => break other,
        }
    };
    if operation.is_some_and(|s| s == "upgrade") && args.iter().any(|s| s == "--rollback") {
        return Ok(());
    }
    let root = parent.parent().ok_or_else(io_error)?;
    let selected = Selection::read(root)?;
    let binary = root
        .join("versions")
        .join(&selected.current)
        .join(binary_name(platform().0));
    if !binary.is_file() {
        return Err(failure(
            "INSTALLATION_INVALID",
            "当前程序缺失；请执行 dt-cli upgrade --rollback。",
        ));
    }
    let mut command = Command::new(&binary);
    command.args(args);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let _ = command.exec();
        Err(failure(
            "INSTALLED_PROGRAM_UNAVAILABLE",
            "当前程序无法启动；请执行 dt-cli upgrade --rollback。",
        ))
    }
    #[cfg(not(unix))]
    {
        let status = command.status().map_err(|_| {
            failure(
                "INSTALLED_PROGRAM_UNAVAILABLE",
                "当前程序无法启动；请执行 dt-cli upgrade --rollback。",
            )
        })?;
        std::process::exit(status.code().unwrap_or(1));
    }
}

#[cfg(test)]
mod tests;
