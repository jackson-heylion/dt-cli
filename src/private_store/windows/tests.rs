use super::*;
use serde_json::{Value, json};

#[test]
fn metadata_roundtrip_keeps_private_owner_and_acl_after_atomic_replacement() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("records/run.json");
    for revision in [1, 2] {
        crate::private_store::write(&path, &json!({"revision":revision})).unwrap();
        validate(path.parent().unwrap()).unwrap();
        validate(&path).unwrap();
        let read: Value = crate::private_store::read(&path).unwrap().unwrap();
        assert_eq!(read["revision"], revision);
    }
}
fn everyone_acl() -> Local {
    let mut sid = [0usize; 16];
    let mut size = std::mem::size_of_val(&sid) as u32;
    unsafe {
        assert_ne!(
            Security::CreateWellKnownSid(
                Security::WinWorldSid,
                null_mut(),
                sid.as_mut_ptr().cast(),
                &mut size
            ),
            0
        );
        let entry = EXPLICIT_ACCESS_W {
            grfAccessPermissions: FILE_ALL_ACCESS,
            grfAccessMode: SET_ACCESS,
            Trustee: TRUSTEE_W {
                TrusteeForm: TRUSTEE_IS_SID,
                ptstrName: sid.as_mut_ptr().cast(),
                ..Default::default()
            },
            ..Default::default()
        };
        let mut acl = null_mut();
        assert_eq!(SetEntriesInAclW(1, &entry, null(), &mut acl), 0);
        Local(acl.cast())
    }
}
#[test]
fn broad_or_null_acl_is_rejected_and_never_repaired_during_read_or_write() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("records/run.json");
    crate::private_store::write(&path, &json!({"original":true})).unwrap();
    let file = open(&path, READ_CONTROL | WRITE_DAC).unwrap();
    let broad = everyone_acl();
    for dacl in [broad.0.cast::<ACL>(), null_mut()] {
        unsafe {
            assert_eq!(
                SetSecurityInfo(
                    file.as_raw_handle().cast(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    null_mut(),
                    null_mut(),
                    dacl,
                    null()
                ),
                0
            );
        }
        assert!(crate::private_store::read::<Value>(&path).is_err());
        assert!(crate::private_store::write(&path, &json!({"replaced":true})).is_err());
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(&path).unwrap()).unwrap(),
            json!({"original":true})
        );
    }
}
#[test]
fn reparse_directory_is_rejected_without_following_it() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target");
    let link = root.path().join("link");
    create_directory(&target).unwrap();
    // Directory junctions require neither Developer Mode nor the symlink privilege.
    let output = std::process::Command::new("cmd.exe")
        .args(["/c", "mklink", "/J"])
        .arg(&link)
        .arg(&target)
        .output()
        .unwrap();
    assert!(output.status.success(), "native junction setup failed");
    assert!(crate::private_store::validate(&link, true).is_err());
    assert!(!target.join("run.json").exists());
    std::fs::remove_dir(link).unwrap();
}
