use super::*;

pub(super) struct Consent {
    pub(super) iams: Vec<(String, Arc<Mutex<Fake>>)>,
    pub(super) deny: bool,
}
impl Browser for Consent {
    fn open(&self, address: &str) -> Result<()> {
        if let Some((_, fake)) = self
            .iams
            .iter()
            .find(|(origin, _)| address.starts_with(&format!("{origin}/cli-governed-intent.html?")))
        {
            let url = url::Url::parse(address).unwrap();
            let id = url
                .query_pairs()
                .find(|(key, _)| key == "intentId")
                .unwrap()
                .1
                .into_owned();
            let mut state = fake.lock().unwrap();
            let intent = state.intents.get_mut(&id).unwrap();
            intent["state"] = json!(if self.deny { "rejected" } else { "approved" });
            return Ok(());
        }
        let (origin, fake) = self
            .iams
            .iter()
            .find(|(origin, _)| address.starts_with(&format!("{origin}/cli-api/oauth2/authorize?")))
            .expect("authorize URL of a known fake IAM");
        let url = url::Url::parse(address).unwrap();
        let fields: BTreeMap<String, String> = url.query_pairs().into_owned().collect();
        let code = {
            let mut f = fake.lock().unwrap();
            f.authorize.push(fields.clone());
            f.challenge = fields.get("code_challenge").cloned();
            f.counter += 1;
            f.code = format!("code-{}", f.counter);
            f.code.clone()
        };
        let mut redirect = url::Url::parse(&fields["redirect_uri"]).unwrap();
        redirect
            .query_pairs_mut()
            .append_pair("state", &fields["state"])
            .append_pair("iss", origin);
        if self.deny {
            redirect
                .query_pairs_mut()
                .append_pair("error", "access_denied");
        } else {
            redirect.query_pairs_mut().append_pair("code", &code);
        }
        tokio::spawn(async move {
            let _ = reqwest::Client::new().get(redirect).send().await;
        });
        Ok(())
    }
}
pub(super) struct NoBrowser;
impl Browser for NoBrowser {
    fn open(&self, _: &str) -> Result<()> {
        panic!("this command must not open a browser")
    }
}

#[derive(Clone, Default)]
pub(super) struct MemoryStore(
    pub(super) Arc<Mutex<BTreeMap<String, String>>>,
    pub(super) Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
);
impl MemoryStore {
    pub(super) fn keys(&self) -> Vec<String> {
        self.0.lock().unwrap().keys().cloned().collect()
    }
    pub(super) fn get(&self, key: &str) -> Credentials {
        self.read(key).unwrap().unwrap()
    }
    pub(super) fn governed_key(&self) -> String {
        let keys: Vec<_> = self
            .keys()
            .into_iter()
            .filter(|key| key.starts_with("governed/"))
            .collect();
        assert_eq!(keys.len(), 1, "{keys:?}");
        keys[0].clone()
    }
}
impl CredentialStore for MemoryStore {
    fn read_integrity_key(&self, key: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.1.lock().unwrap().get(key).cloned())
    }
    fn write_integrity_key(&self, key: &str, bytes: &[u8]) -> Result<()> {
        self.1.lock().unwrap().insert(key.into(), bytes.to_vec());
        Ok(())
    }
    fn read(&self, key: &str) -> Result<Option<Credentials>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .get(key)
            .map(|value| serde_json::from_str(value).unwrap()))
    }
    fn write(&self, key: &str, credentials: &Credentials) -> Result<()> {
        self.0
            .lock()
            .unwrap()
            .insert(key.into(), serde_json::to_string(credentials).unwrap());
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}
pub(super) struct ForbiddenStore;
impl CredentialStore for ForbiddenStore {
    fn read(&self, _: &str) -> Result<Option<Credentials>> {
        panic!("this command must not read credentials")
    }
    fn write(&self, _: &str, _: &Credentials) -> Result<()> {
        panic!("this command must not write credentials")
    }
    fn delete(&self, _: &str) -> Result<()> {
        panic!("this command must not delete credentials")
    }
}

pub(super) fn environment(origin: &str) -> Environment {
    Environment {
        api_origin: origin.into(),
        portal_origin: "https://portal.fixture.invalid".into(),
        recovery_url: "https://portal.fixture.invalid/cli-authorizations.html".into(),
        launch_path: "/cli/launch".into(),
    }
}

pub(super) struct Harness {
    pub(super) root: tempfile::TempDir,
    pub(super) rt: Runtime,
    pub(super) store: MemoryStore,
    pub(super) iams: Vec<(String, Arc<Mutex<Fake>>)>,
}
impl Harness {
    pub(super) fn new(iams: &[(&str, &Iam)]) -> Self {
        Self::build(iams, false, true)
    }
    pub(super) fn build(iams: &[(&str, &Iam)], deny: bool, interactive: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();
        let shared: Vec<_> = iams
            .iter()
            .map(|(_, iam)| (iam.origin.clone(), iam.fake.clone()))
            .collect();
        let rt = Runtime {
            root: root.path().into(),
            environments: iams
                .iter()
                .map(|(name, iam)| (name.to_string(), environment(&iam.origin)))
                .collect(),
            store: Box::new(store.clone()),
            browser: Box::new(Consent {
                iams: shared.clone(),
                deny,
            }),
            interactive,
            aggregate_budget: Duration::from_secs(5),
        };
        Self {
            root,
            rt,
            store,
            iams: shared,
        }
    }
    /// The same local files, but no credential store and no browser at all.
    pub(super) fn sealed(&self) -> Runtime {
        Runtime {
            root: self.root.path().into(),
            environments: self.rt.environments.clone(),
            store: Box::new(ForbiddenStore),
            browser: Box::new(NoBrowser),
            interactive: true,
            aggregate_budget: Duration::from_secs(5),
        }
    }
    pub(super) fn file(&self, name: &str) -> std::path::PathBuf {
        self.root.path().join("governed").join(name)
    }
    /// Every issued credential and the legacy tokens: none may appear in any output.
    pub(super) fn secrets(&self) -> Vec<String> {
        let mut all = vec![OLD_ACCESS.to_owned(), OLD_REFRESH.to_owned()];
        for (_, fake) in &self.iams {
            let fake = fake.lock().unwrap();
            all.extend(fake.issued.iter().cloned());
        }
        all
    }
    pub(super) async fn run_with(&self, rt: &Runtime, args: &[&str]) -> (Value, u8) {
        let (value, exit, _) = dt_cli::execute(
            rt,
            std::iter::once("dt-cli")
                .chain(args.iter().copied())
                .map(str::to_owned)
                .collect(),
        )
        .await;
        let text = value.to_string();
        for secret in self.secrets() {
            assert!(
                !text.contains(&secret),
                "{args:?} leaked a credential: {text}"
            );
        }
        for fragment in [
            "dtcli_g_a_",
            "dtcli_g_r_",
            "dtcli_g_s_",
            "dtcli_a_",
            "dtcli_r_",
        ] {
            assert!(
                !text.contains(fragment),
                "{args:?} leaked {fragment}: {text}"
            );
        }
        (value, exit)
    }
    pub(super) async fn run(&self, args: &[&str]) -> (Value, u8) {
        self.run_with(&self.rt, args).await
    }
    pub(super) async fn login(&self, name: &str, environment: &str, system: &str) -> Value {
        let (value, exit) = self
            .run(&[
                "auth",
                "login",
                "--profile",
                name,
                "--environment",
                environment,
                "--system",
                system,
            ])
            .await;
        assert_eq!(exit, 0, "{value}");
        value
    }
}
pub(super) fn supply() -> Iam {
    Iam::start(SYSTEM, ENV, supply_operations())
}
pub(super) fn assert_failure(value: &Value, exit: u8, expected_exit: u8, code: &str) {
    assert_eq!(exit, expected_exit, "{value}");
    assert_eq!(value["ok"], false, "{value}");
    assert_eq!(value["error"]["code"], code, "{value}");
}

// A2 登录/目录：新 provider profile、独立凭证键、目录缓存绑定授权。
