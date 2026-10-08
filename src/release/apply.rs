use super::*;

pub(super) fn apply(
    root: &Path,
    package: Option<Package>,
    options: &Options<'_>,
    host: (&str, &str),
    verifier: &dyn Verifier,
) -> Result<Value> {
    if options.prune
        && (package.is_some()
            || options.install
            || options.rollback
            || options.candidate
            || options.expected.is_some()
            || options.signer.is_some())
    {
        return Err(invalid());
    }
    if options.rollback
        && (package.is_some()
            || options.install
            || options.candidate
            || options.expected.is_some()
            || options.signer.is_some())
        || options.install && (package.is_none() || options.check)
    {
        return Err(invalid());
    }
    if options.check && package.is_none() && !options.rollback && !options.prune {
        return status(root, host);
    }
    let _lock = if options.check {
        None
    } else {
        Some(lock(root)?)
    };
    let installed = root.join("trust.json").exists();
    if !installed && !options.install {
        return Err(failure(
            "INSTALLATION_NOT_MANAGED",
            "当前程序尚未由内置安装器管理；请从可信程序包执行 install。",
        ));
    }
    if installed && options.install {
        return Err(failure(
            "ALREADY_INSTALLED",
            "该目录已有受管安装；请使用 upgrade。",
        ));
    }
    let selected = if installed {
        Some(Selection::read(root)?)
    } else {
        None
    };
    let trust = if installed {
        trust_at(root)?
    } else if options.candidate {
        if options.expected.is_none() {
            return Err(failure(
                "CANDIDATE_CHECKSUM_REQUIRED",
                "候选安装必须提供组织可信入口公布的整个 ZIP 摘要。",
            ));
        }
        Trust {
            schema_version: SCHEMA,
            release_type: "candidate".into(),
            identity: None,
        }
    } else {
        Trust {
            schema_version: SCHEMA,
            release_type: "release".into(),
            identity: None,
        }
    };
    if options.prune {
        return prune(root, selected.as_ref().unwrap(), host, options.check);
    }
    if options.rollback {
        let selected = selected.unwrap();
        let previous = selected
            .previous
            .ok_or_else(|| failure("ROLLBACK_UNAVAILABLE", "没有已验证的前一版本。"))?;
        let (manifest, binary) = manifest_at(root, &previous, host)?;
        verify_payload(&manifest, &binary, &trust, verifier)?;
        if !options.check {
            Selection {
                current: previous,
                previous: Some(selected.current),
            }
            .write(root)?;
        }
        return Ok(
            json!({"action":if options.check{"rollback-check"}else{"rollback"},"version":manifest.version,"changed":!options.check,"configurationChanged":false,"credentialsChanged":false}),
        );
    }
    let package = package.ok_or_else(invalid)?;
    if trust.release_type == "release" && options.expected.is_none() {
        return Err(failure(
            "PACKAGE_CHECKSUM_REQUIRED",
            "内部程序包安装和升级必须提供可信入口公布的整个 ZIP --sha256。",
        ));
    }
    if trust.release_type == "candidate" && (!options.candidate || options.expected.is_none()) {
        return Err(failure(
            "CANDIDATE_CHECKSUM_REQUIRED",
            "候选升级必须显式指定 --candidate 和 ZIP 的可信 --sha256。",
        ));
    }
    if trust.release_type == "release" && options.candidate {
        return Err(failure(
            "RELEASE_CHANNEL_MISMATCH",
            "正式安装不能接受候选程序包。",
        ));
    }
    if package.manifest.release_type != trust.release_type {
        return Err(failure(
            "RELEASE_CHANNEL_MISMATCH",
            "程序包与安装目录的信任方式不一致。",
        ));
    }
    if let Some(selected) = &selected {
        let (current, _) = manifest_at(root, &selected.current, host)?;
        if trust.release_type == "release"
            && package.manifest.sha256 != current.sha256
            && version(&package.manifest.version)? <= version(&current.version)?
        {
            return Err(failure(
                "DOWNGRADE_BLOCKED",
                "正式升级只接受更高版本；需要回退时使用 --rollback。",
            ));
        }
    }
    let stage = tempfile::tempdir().map_err(|_| io_error())?;
    let staged = stage.path().join(&package.manifest.binary);
    fs::write(&staged, &package.bytes).map_err(|_| io_error())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&staged, fs::Permissions::from_mode(0o755)).map_err(|_| io_error())?;
    }
    verify_payload(&package.manifest, &staged, &trust, verifier)?;
    if options.check {
        return Ok(
            json!({"action":"upgrade-check","packageVerified":true,"version":package.manifest.version,"archiveSha256":package.archive_digest,"updateAvailable":selected.as_ref().is_none_or(|s|s.current!=package.manifest.sha256),"changed":false,"httpRequests":0}),
        );
    }
    if selected
        .as_ref()
        .is_some_and(|s| s.current == package.manifest.sha256)
    {
        return Ok(json!({"action":"upgrade","version":package.manifest.version,"changed":false}));
    }
    if !installed {
        // Claim only an empty installation root; no unrelated files or existing launchers are overwritten.
        if root.exists() && fs::read_dir(root).map_err(|_| io_error())?.next().is_some() {
            return Err(failure(
                "INSTALLATION_DIRECTORY_OCCUPIED",
                "安装目录包含已有文件；请选择独立空目录。",
            ));
        }
    }
    // Prepare an entire fresh installation off to the side; interruption before rename is safely retryable.
    let fresh = if installed {
        None
    } else {
        Some(tempfile::tempdir_in(root.parent().ok_or_else(io_error)?).map_err(|_| io_error())?)
    };
    let working = fresh.as_ref().map(|dir| dir.path()).unwrap_or(root);
    plain_directory(&working.join("versions"))?;
    plain_directory(&working.join("bin"))?;
    let target = working.join("versions").join(&package.manifest.sha256);
    if target.exists() {
        let (existing, binary) = manifest_at(working, &package.manifest.sha256, host)?;
        if existing != package.manifest {
            return Err(bad_package());
        }
        verify_payload(&existing, &binary, &trust, verifier)?;
    } else {
        let folder = tempfile::tempdir_in(working.join("versions")).map_err(|_| io_error())?;
        fs::copy(&staged, folder.path().join(&package.manifest.binary)).map_err(|_| io_error())?;
        // Windows FlushFileBuffers requires a writable handle, including for a freshly copied file.
        OpenOptions::new()
            .write(true)
            .open(folder.path().join(&package.manifest.binary))
            .and_then(|f| f.sync_all())
            .map_err(|_| io_error())?;
        atomic_write(
            &folder.path().join("manifest.json"),
            &serde_json::to_vec(&package.manifest).map_err(|_| bad_package())?,
        )?;
        fs::rename(folder.path(), &target).map_err(|_| io_error())?;
    }
    if !installed {
        // The immutable native launcher delegates via argv, also on Windows. It is never overwritten by an upgrade.
        let launcher = working.join("bin").join(binary_name(host.0));
        fs::copy(&staged, &launcher).map_err(|_| io_error())?;
        atomic_write(
            &working.join("bin").join(MARKER),
            &serde_json::to_vec(&Launcher {
                schema_version: SCHEMA,
            })
            .unwrap(),
        )?;
        atomic_write(
            &working.join("trust.json"),
            &serde_json::to_vec(&trust).unwrap(),
        )?;
    }
    Selection {
        current: package.manifest.sha256.clone(),
        previous: selected.map(|s| s.current),
    }
    .write(working)?;
    if !installed {
        if root.exists() {
            fs::remove_dir(root).map_err(|_| io_error())?;
        }
        fs::rename(working, root).map_err(|_| io_error())?;
    }
    Ok(
        json!({"action":if options.install{"install"}else{"upgrade"},"version":package.manifest.version,"releaseType":trust.release_type,"changed":true,"launcher":root.join("bin").join(binary_name(host.0)),"pathDirectory":root.join("bin"),"configurationChanged":false,"credentialsChanged":false,"skillsChanged":false}),
    )
}
