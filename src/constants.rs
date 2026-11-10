//! Fixed protocol contracts and resource budgets, grouped by purpose.
//! Business formulas live in their own skill. Deployment settings remain in their existing configuration files.

pub(crate) mod identity {
    pub const CLIENT_ID: &str = "dt-cli";
    pub const ORGANIZATION: &str = "datousoft";
    pub const QUALIFIER: &str = "com";
    pub const RANDOM_BYTES: usize = 32;
    pub const DIGEST_LENGTH: usize = 64;
    pub const COMMIT_LENGTH: usize = 40;
    pub const MAX_ID_LENGTH: usize = 80;
    pub const CALLBACK_REQUEST_BYTES: usize = 8192;
    pub const CALLBACK_BUFFER_BYTES: usize = 512;
    pub const CALLBACK_ERROR_LENGTH: usize = 256;
}

pub(crate) mod protocol {
    pub const ENVELOPE_SCHEMA: &str = "1.0";
    pub const FILE_SCHEMA: u32 = 1;
}

pub(crate) mod query {
    pub const CONNECT_SECONDS: u64 = 3;
    pub const REQUEST_SECONDS: u64 = 10;
}

pub(crate) mod storage {
    pub const METADATA_BYTES: u64 = 16 * 1024;
    pub const LOCK_SECONDS: u64 = 10;
    pub const JOURNAL_LOCK_SECONDS: u64 = 5;
    pub const LOCK_POLL_MILLIS: u64 = 50;
    pub const JOURNAL_SIGNING_KEY: &str = "metadata/workpack-signing-v1";
    pub const CREDENTIAL_DIRECTORY: &str = "credentials";
    pub const EXECUTABLE_MODE: u32 = 0o700;
    pub const NON_OWNER_PERMISSION_MASK: u32 = 0o077;
    pub const PRIVATE_FILE_MODE: u32 = 0o600;
    pub const FILE_KIND_MASK: u32 = 0o170000;
    pub const REGULAR_FILE_KIND: u32 = 0o100000;
    pub const DIRECTORY_KIND: u32 = 0o040000;
    pub const EXECUTABLE_MASK: u32 = 0o111;
    pub const PORTABLE_FILE_MODE: u32 = 0o100644;
    #[cfg(windows)]
    pub const WINDOWS_IDENTITY_BYTES: u32 = 65536;
    #[cfg(windows)]
    pub const WINDOWS_MAX_ACCESS_ENTRIES: u32 = 64;
}

pub(crate) mod distribution {
    pub const MAX_BINARY_BYTES: u64 = 256 * 1024 * 1024;
    pub const MAX_JSON_BYTES: u64 = 16 * 1024;
    pub const MAX_SKILL_BYTES: u64 = 16 * 1024 * 1024;
    pub const MAX_SKILL_FILE_BYTES: u64 = 1024 * 1024;
    pub const MAX_SKILL_FILES: usize = 128;
    pub const MAX_SKILL_NAME_LENGTH: usize = 63;
    pub const MAX_OBJECT_KEY_LENGTH: usize = 200;
    pub const MAX_SOURCE_VERSION_LENGTH: usize = 200;
    pub const NATIVE_CACHE_FILE: &str = "distribution-cache.json";
    pub const MANIFEST_FILE: &str = "manifest.json";
    pub const TRUST_FILE: &str = "trust.json";
    pub const SKILL_ENTRY_FILE: &str = "SKILL.md";
    pub const MAX_INDEX_SKILL_BYTES: u64 = MAX_JSON_BYTES * 64;
    pub const INSTALL_RECORD_BYTES: u64 = 64 * 1024;
    pub const CONNECT_SECONDS: u64 = 5;
    pub const DOWNLOAD_SECONDS: u64 = 90;
    pub const UPDATE_SECONDS: u64 = 120;
    pub const PROBE_SECONDS: u64 = 20;
    pub const PROCESS_POLL_MILLIS: u64 = 10;
    pub const PROBE_BYTES: u64 = 64 * 1024;
    pub const CACHE_SECONDS: i64 = 86_400;
    pub const LAUNCHER_MARKER: &str = "dt-cli.launcher.json";
    pub const SKILL_RECORD: &str = ".dt-cli-skill-install.json";
    pub const NATIVE_STABLE: &str = "channels/native-stable.json";
    pub const UNIVERSAL_SKILL_STABLE: &str = "channels/skill-stable.json";
}

pub(crate) mod clients {
    pub const WORKBUDDY_DIRECTORY: &str = ".workbuddy";
    pub const WORKBUDDY_SETTINGS: &str = "settings.json";
    pub const WORKBUDDY_SETTINGS_BYTES: u64 = 1024 * 1024;
}
