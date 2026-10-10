use super::configure;
use serde_json::json;
use std::{fs, path::PathBuf};

fn run(
    home: &std::path::Path,
    root: &std::path::Path,
    fix: bool,
) -> super::Result<serde_json::Value> {
    configure(home, &[root.to_owned()], fix)
}

fn fixture(value: &serde_json::Value) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let home = tempfile::tempdir().unwrap();
    let parent = home.path().join(".workbuddy");
    fs::create_dir(&parent).unwrap();
    let settings = parent.join("settings.json");
    fs::write(&settings, serde_json::to_vec(value).unwrap()).unwrap();
    let root = home
        .path()
        .join("Library/Application Support/com.datousoft.dt-cli");
    (home, settings, root)
}

#[test]
fn preview_does_not_write_and_explicit_fix_preserves_other_settings() {
    let original = json!({
        "enabledPlugins":{"other":true},
        "sandbox":{
            "extraAllowWrite":["~/.dws"],
            "orderedRules":{"file":{"rules":[{"path":"~/.ssh/","write":"ask"}]}}
        }
    });
    let (home, settings, root) = fixture(&original);
    let before = fs::read(&settings).unwrap();
    let preview = run(home.path(), &root, false).unwrap();
    assert_eq!(preview["configured"], false);
    assert_eq!(preview["changed"], false);
    assert_eq!(fs::read(&settings).unwrap(), before);
    assert!(!root.exists());
    let fixed = run(home.path(), &root, true).unwrap();
    assert_eq!(fixed["configured"], true);
    assert_eq!(fixed["changed"], true);
    assert_eq!(fixed["storageVerified"], false);
    assert_eq!(fixed["settingsKey"], "sandbox.extraAllowWrite");
    assert_eq!(fixed["nextAction"], "restart-workbuddy-then-verify");
    let after: serde_json::Value = serde_json::from_slice(&fs::read(&settings).unwrap()).unwrap();
    assert_eq!(after["enabledPlugins"], original["enabledPlugins"]);
    assert_eq!(
        after["sandbox"]["orderedRules"],
        original["sandbox"]["orderedRules"]
    );
    assert_eq!(after["sandbox"]["extraAllowWrite"], json!(["~/.dws", root]));
    assert!(!root.exists());
    let fixed_bytes = fs::read(&settings).unwrap();
    assert_eq!(run(home.path(), &root, true).unwrap()["changed"], false);
    assert_eq!(fs::read(&settings).unwrap(), fixed_bytes);
    assert_eq!(fs::read_dir(settings.parent().unwrap()).unwrap().count(), 1);
}

#[test]
fn existing_home_relative_directory_is_recognized_without_rewriting() {
    let (home, settings, root) = fixture(&json!({
        "sandbox":{"extraAllowWrite":["~/Library/Application Support/com.datousoft.dt-cli/"]}
    }));
    let before = fs::read(&settings).unwrap();
    assert_eq!(run(home.path(), &root, true).unwrap()["changed"], false);
    assert_eq!(fs::read(&settings).unwrap(), before);
}

#[test]
fn malformed_or_linked_settings_are_retained() {
    for value in [
        json!([]),
        json!({"sandbox":false}),
        json!({"sandbox":{"extraAllowWrite":[7]}}),
    ] {
        let (home, settings, root) = fixture(&value);
        let before = fs::read(&settings).unwrap();
        assert!(run(home.path(), &root, true).is_err());
        assert_eq!(fs::read(&settings).unwrap(), before);
    }
    let (home, settings, root) = fixture(&json!({}));
    fs::write(&settings, r#"{"sandbox":{},"sandbox":{}}"#).unwrap();
    let before = fs::read(&settings).unwrap();
    assert!(run(home.path(), &root, true).is_err());
    assert_eq!(fs::read(&settings).unwrap(), before);
    #[cfg(unix)]
    {
        let other = home.path().join("other-settings.json");
        fs::rename(&settings, &other).unwrap();
        std::os::unix::fs::symlink(&other, &settings).unwrap();
        assert!(run(home.path(), &root, true).is_err());
        assert_eq!(fs::read(&other).unwrap(), before);
    }
}

#[test]
fn distinct_account_and_installation_directories_are_added_once() {
    let (home, settings, root) = fixture(&json!({}));
    let installation = home.path().join("Local/datousoft/dt-cli/data");
    let roots = [root.clone(), installation.clone()];
    let result = configure(home.path(), &roots, true).unwrap();
    assert_eq!(result["allowDirectories"], json!(roots));
    let value: serde_json::Value = serde_json::from_slice(&fs::read(&settings).unwrap()).unwrap();
    assert_eq!(value["sandbox"]["extraAllowWrite"], json!(roots));
    assert_eq!(
        configure(home.path(), &roots, true).unwrap()["changed"],
        false
    );
    assert!(!installation.exists());
}

#[test]
fn utf8_bom_settings_are_read_without_losing_existing_fields() {
    let (home, settings, root) = fixture(&json!({}));
    fs::write(&settings, "\u{feff}{\"other\":true}").unwrap();
    assert_eq!(run(home.path(), &root, true).unwrap()["configured"], true);
    let value: serde_json::Value = serde_json::from_slice(&fs::read(&settings).unwrap()).unwrap();
    assert_eq!(value["other"], true);
}

#[cfg(windows)]
#[test]
fn windows_case_and_separators_match_and_acl_is_preserved() {
    use std::process::Command;
    let (home, settings, root) = fixture(&json!({}));
    let acl = || {
        let output = Command::new("icacls").arg(&settings).output().unwrap();
        assert!(output.status.success());
        // A same-directory Windows move can reclassify the origin of unchanged ACEs.
        // Keep every principal, access bit, order and duplicate; only ignore origin/spacing.
        String::from_utf8_lossy(&output.stdout)
            .replace("(I)", "")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    let protected =
        || {
            let output = Command::new("powershell").args(["-NoProfile", "-Command",
            "$p=$env:DT_CLI_TEST_SETTINGS; (Get-Acl -LiteralPath $p).AreAccessRulesProtected"])
            .env("DT_CLI_TEST_SETTINGS", &settings).output().unwrap();
            assert!(output.status.success());
            output.stdout
        };
    let before = acl();
    let protection_before = protected();
    run(home.path(), &root, true).unwrap();
    assert_eq!(acl(), before);
    assert_eq!(protected(), protection_before);
    let altered = root
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_uppercase()
        + "/";
    let value = json!({"sandbox":{"extraAllowWrite":[altered]}});
    fs::write(&settings, serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(run(home.path(), &root, true).unwrap()["changed"], false);
    assert!(!super::matches_directory(home.path(), "C:/", &root));
}

#[cfg(windows)]
#[test]
fn windows_rejects_settings_writable_by_everyone_without_replacing_them() {
    let (home, settings, root) = fixture(&json!({"keep":"original"}));
    let before = fs::read(&settings).unwrap();
    let result = std::process::Command::new("icacls")
        .arg(&settings)
        .args(["/grant", "*S-1-1-0:(W)"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(run(home.path(), &root, true).is_err());
    assert_eq!(fs::read(&settings).unwrap(), before);
}
