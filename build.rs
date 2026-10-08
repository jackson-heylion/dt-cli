use sha2::{Digest, Sha256};
use std::process::Command;
fn main() {
    // A worktree's .git is a file; resolve the actual metadata and the current branch ref.
    let mut paths = vec![
        "HEAD".to_owned(),
        "index".to_owned(),
        "packed-refs".to_owned(),
    ];
    if let Some(branch) = git(&["symbolic-ref", "-q", "HEAD"]) {
        paths.push(branch);
    }
    for path in paths {
        if let Some(path) = git(&["rev-parse", "--git-path", &path])
            && std::path::Path::new(&path).exists()
        {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=Cargo.lock");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=catalog");
    println!("cargo:rerun-if-changed=skills/dt-cli/scripts/distribution.json");
    let sha = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_else(|| "unknown".into());
    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .is_none_or(|o| !o.status.success() || !o.stdout.is_empty());
    let commit = format!("{sha}{}", if dirty { "+dirty" } else { "" });
    println!("cargo:rustc-env=BUILD_COMMIT={commit}");
    let catalog_bytes = std::fs::read("catalog/v1.json").expect("release catalog");
    let catalog: serde_json::Value =
        serde_json::from_slice(&catalog_bytes).expect("release catalog");
    let metadata = serde_json::json!({
        "cliVersion": std::env::var("CARGO_PKG_VERSION").unwrap(),
        "buildCommit": commit,
        "buildTarget": std::env::var("TARGET").unwrap(),
        "localDevelopment": std::env::var_os("CARGO_FEATURE_LOCAL_DEV").is_some(),
        "catalogVersion": catalog["version"],
        "catalogDigest": format!("{:x}", Sha256::digest(&catalog_bytes)),
        "sourceVersion": catalog["sourceVersion"]
    })
    .to_string();
    println!("cargo:rustc-env=BUILD_METADATA={metadata}");
    // OUT_DIR is <profile>/build/<package>/out, including cross-target builds.
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let profile = out.ancestors().nth(3).expect("Cargo profile directory");
    std::fs::write(profile.join("dt-cli.build.json"), metadata + "\n").expect("build metadata");
}

fn git(args: &[&str]) -> Option<String> {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}
