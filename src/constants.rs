//! Fixed protocol contracts and resource budgets, grouped by purpose.
//! Business formulas live in their own skill. Deployment settings remain in their existing configuration files.

pub(crate) mod identity {
    pub const CLIENT_ID: &str = "dt-cli";
    pub const SUPPLY_CHAIN_SYSTEM: &str = "supply-chain-server";
    pub const PROVIDER: &str = "governed";
    pub const ORGANIZATION: &str = "datousoft";
    pub const QUALIFIER: &str = "com";
    pub const RANDOM_BYTES: usize = 32;
    pub const HANDLE_LENGTH: usize = 43;
    pub const DIGEST_LENGTH: usize = 64;
    pub const COMMIT_LENGTH: usize = 40;
    pub const MAX_ID_LENGTH: usize = 80;
    pub const MAX_HANDLE_LENGTH: usize = 128;
    pub const MAX_VERSION_LENGTH: usize = 40;
    pub const ACCESS_SECONDS: i64 = 43_200;
    pub const SYSTEM_TOKEN_SECONDS: i64 = 300;
    pub const CLOCK_SKEW_SECONDS: i64 = 120;
    pub const REFRESH_AHEAD_SECONDS: i64 = 30;
    pub const BROWSER_WAIT_SECONDS: u64 = 900;
    pub const CALLBACK_REQUEST_BYTES: usize = 8192;
    pub const CALLBACK_BUFFER_BYTES: usize = 512;
    pub const CALLBACK_ERROR_LENGTH: usize = 256;
    pub const IDEMPOTENCY_MIN_LENGTH: usize = 16;
    pub const ACCESS_PREFIX: &str = "dtcli_g_a_";
    pub const REFRESH_PREFIX: &str = "dtcli_g_r_";
    pub const SYSTEM_PREFIX: &str = "dtcli_g_s_";
    pub const PERSONAL_ACCESS_PREFIX: &str = "dtcli_a_";
    pub const PERSONAL_REFRESH_PREFIX: &str = "dtcli_r_";
}

pub(crate) mod protocol {
    pub const ENVELOPE_SCHEMA: &str = "1.0";
    pub const CONTRACT_SCHEMA_VERSION: u64 = 1;
    pub const FILE_SCHEMA: u32 = 1;
    pub const AUTHORIZE_ROUTE: &str = "/cli-api/oauth2/authorize";
    pub const TOKEN_ROUTE: &str = "/cli-api/oauth2/token";
    pub const EXCHANGE_ROUTE: &str = "/cli-api/oauth2/exchange";
    pub const REVOKE_ROUTE: &str = "/cli-api/oauth2/revoke";
    pub const CATALOG_ROUTE: &str = "/cli-api/v1/catalog";
    pub const DELIVERY_CENTERS_LIST: &str = "supply-chain-server.delivery-centers.list";
    pub const JOBS_ROUTE: &str = "/cli-api/v1/jobs";
    pub const DIRECT_CREDENTIAL_SUFFIX: &str = "direct-credential";
    pub const LOCAL_JOB_PREFIX: &str = "local_";
    pub const VIEWS_SUFFIX: &str = "views";
    pub const INTENTS_ROUTE: &str = "/cli-api/v1/intents";
    pub const INTENT_PAGE: &str = "/cli-governed-intent.html?intentId=";
    pub const TOKEN_EXCHANGE: &str = "urn:ietf:params:oauth:grant-type:token-exchange";
    pub const ACCESS_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:access_token";
    pub const WORKFLOW_QUERY_ROUTE: &str = "/cli/v1/workflow/messages/query";
    pub const WORKFLOW_LIST_OPERATION: &str = "iam.workflow.list";
    pub const MAX_SAFE_JSON_INTEGER: u64 = 9_007_199_254_740_991;
    pub const MESSAGE_ID_LENGTH: usize = 19;
    pub const MAX_OPERATION_LENGTH: usize = 180;
    pub const MAX_TRACE_LENGTH: usize = 64;
    pub const MAX_META_STATE_LENGTH: usize = 16;
    pub const MAX_META_TEXT_LENGTH: usize = 256;
}

pub(crate) mod query {
    pub const CONNECT_SECONDS: u64 = 3;
    pub const REQUEST_SECONDS: u64 = 10;
    pub const READ_RETRY_SECONDS: u64 = 60;
    pub const AGGREGATE_SECONDS: u64 = 30;
    pub const RESULT_TIMEOUT_SECONDS: u64 = 90;
    pub const MAX_WAIT_SECONDS: u64 = 600;
    pub const INITIAL_POLL_SECONDS: u64 = 2;
    pub const MAX_POLL_SECONDS: u64 = 10;
    pub const MAX_RETRY_AFTER_SECONDS: u64 = 3600;
    pub const DEFAULT_PAGE_SIZE: u32 = 50;
    pub const BUSINESS_PAGE_SIZE: u32 = 100;
    pub const SCOPE_MAX_PAGES: u32 = 100;
    pub const SCOPE_TREE_DEPTH: u32 = 16;
    pub const WORKFLOW_MAX_PAGE_SIZE: u32 = 200;
    pub const INTENT_POLL_SECONDS: u64 = 3;
    pub const MAX_PAGES: u32 = 20;
    pub const MAX_ITEMS: usize = 1000;
    pub const DISCOVERY_LIMIT: usize = 100;
    pub const PAGE_BYTES: usize = 2 * 1024 * 1024;
    pub const AGGREGATE_BYTES: usize = 10 * 1024 * 1024;
    pub const RESULT_BYTES: usize = AGGREGATE_BYTES + 64 * 1024;
    pub const ARGUMENT_BYTES: usize = 64 * 1024;
    pub const DIRECT_MAX_PAGES: u32 = 1000;
    pub const DIRECT_MAX_RECORDS: usize = 100_000;
    pub const DIRECT_MAX_SECONDS: u64 = 600;
    pub const DIRECT_REQUEST_INTERVAL_MILLIS: u64 = 200;
    pub const DIRECT_MAX_REQUEST_INTERVAL_MILLIS: u64 = 10_000;
    pub const DIRECT_CURSOR_BYTES: usize = 4096;
    pub const BUSINESS_CREDENTIAL_MAX_SECONDS: i64 = 65;
}

pub(crate) mod storage {
    pub const TASK_RECORD_SCHEMA: u8 = 1;
    pub const METADATA_BYTES: u64 = 16 * 1024;
    pub const LOCK_SECONDS: u64 = 10;
    pub const JOURNAL_LOCK_SECONDS: u64 = 5;
    pub const LOCK_POLL_MILLIS: u64 = 50;
    pub const TERMINAL_RETENTION_HOURS: i64 = 24;
    pub const JOURNAL_SIGNING_KEY: &str = "metadata/workpack-signing-v1";
    pub const GOVERNED_DIRECTORY: &str = "governed";
    pub const CREDENTIAL_DIRECTORY: &str = "credentials";
    pub const CONTEXT_DIRECTORY: &str = "context";
    pub const QUERY_CONTEXT_DIRECTORY: &str = "query-context";
    pub const DELIVERY_CONTEXT_SUFFIX: &str = ".delivery.json";
    pub const SELECTION_FILE: &str = "selection.json";
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
    pub const RUN_RECORD_BYTES: u64 = MAX_JSON_BYTES * 16;
    pub const REQUIREMENTS_BYTES: u64 = MAX_JSON_BYTES * 4;
    pub const CONNECT_SECONDS: u64 = 5;
    pub const DOWNLOAD_SECONDS: u64 = 90;
    pub const UPDATE_SECONDS: u64 = 120;
    pub const PROBE_SECONDS: u64 = 20;
    pub const PREPARE_SECONDS: u64 = 300;
    pub const PROCESS_POLL_MILLIS: u64 = 10;
    pub const PROBE_BYTES: u64 = 64 * 1024;
    pub const CACHE_SECONDS: i64 = 86_400;
    pub const LAUNCHER_MARKER: &str = "dt-cli.launcher.json";
    pub const SKILL_RECORD: &str = ".dt-cli-skill-install.json";
    pub const NATIVE_STABLE: &str = "channels/native-stable.json";
    pub const LEGACY_SKILL_STABLE: &str = "channels/skill-stable.json";
    pub const RUN_DIRECTORY: &str = "business-runs";
    pub const SNAPSHOT_DIRECTORY: &str = "resources";
    pub const RUN_RECORD: &str = "run.json";
    pub const REQUIREMENTS_FILE: &str = "scripts/requirements.json";
}

pub(crate) mod clients {
    pub const SUPPORTED_PYTHON_MINIMUM: &str = "3.9";
    pub const WORKBUDDY_DIRECTORY: &str = ".workbuddy";
    pub const WORKBUDDY_SETTINGS: &str = "settings.json";
    pub const WORKBUDDY_SETTINGS_BYTES: u64 = 1024 * 1024;
}
