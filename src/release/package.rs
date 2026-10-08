use super::*;

pub(super) struct Package {
    pub(super) manifest: Manifest,
    pub(super) bytes: Vec<u8>,
    pub(super) archive_digest: String,
}
impl Package {
    pub(super) fn read(path: &Path, expected: Option<&str>, host: (&str, &str)) -> Result<Self> {
        if expected.is_some_and(|s| !digest(s)) {
            return Err(invalid());
        }
        let archive_bytes = read_limited(path, MAX_BINARY)?;
        let archive_digest = sha(&archive_bytes);
        if expected.is_some_and(|s| s != archive_digest) {
            return Err(failure(
                "PACKAGE_CHECKSUM_MISMATCH",
                "程序包与指定的可信摘要不一致。",
            ));
        }
        let mut archive =
            zip::ZipArchive::new(std::io::Cursor::new(archive_bytes)).map_err(|_| bad_package())?;
        if archive.len() < 2 || archive.len() > 4 {
            return Err(bad_package());
        }
        let mut names = BTreeSet::new();
        for i in 0..archive.len() {
            let file = archive.by_index(i).map_err(|_| bad_package())?;
            let name = file.name().to_owned();
            if !names.insert(name.clone())
                || file.is_dir()
                || file.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000)
                || ![
                    "manifest.json",
                    "dt-cli",
                    "dt-cli.exe",
                    "install.sh",
                    "install.ps1",
                ]
                .contains(&name.as_str())
            {
                return Err(bad_package());
            }
        }
        let mut read_entry = |name: &str, limit: u64| -> Result<Vec<u8>> {
            let file = archive.by_name(name).map_err(|_| bad_package())?;
            if file.size() > limit {
                return Err(bad_package());
            }
            let mut bytes = Vec::new();
            file.take(limit + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| bad_package())?;
            if bytes.len() as u64 > limit {
                return Err(bad_package());
            }
            Ok(bytes)
        };
        let manifest: Manifest = serde_json::from_slice(&read_entry("manifest.json", MAX_JSON)?)
            .map_err(|_| bad_package())?;
        manifest.validate(host)?;
        let bytes = read_entry(&manifest.binary, MAX_BINARY)?;
        if sha(&bytes) != manifest.sha256 || bytes.is_empty() {
            return Err(bad_package());
        }
        Ok(Self {
            manifest,
            bytes,
            archive_digest,
        })
    }
}
