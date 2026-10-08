//! One HTTP client and one compiled catalog per task execution; existing executors remain authoritative.
use super::*;

pub(crate) struct TaskSession<'a> {
    rt: &'a Runtime,
    p: GovernedProfile,
    env: &'a profile::Environment,
    cache: Cache,
    pub(crate) operation: Value,
    client: reqwest::Client,
}
impl<'a> TaskSession<'a> {
    pub(crate) fn new(
        rt: &'a Runtime,
        binding: &crate::profiles::Binding,
        id: &str,
        version: Option<&str>,
        effect: &str,
        params: Option<&Value>,
    ) -> Result<Self> {
        if super::binding(rt, &binding.profile)? != *binding {
            return Err(identity_mismatch());
        }
        let p = require_profile(rt, &binding.profile)?;
        let env = rt.environment(&p.environment)?;
        let cache = read_cache(&rt.root, &binding.profile, &p)?.ok_or_else(not_synced)?;
        let operation = select(&cache, id, version)?;
        usable(operation, effect)?;
        if effect == "write"
            && !matches!(
                operation["confirmation"]["channel"].as_str(),
                Some("agent-cli" | "backend-grant")
            )
        {
            return Err(contract_changed());
        }
        if let Some(params) = params {
            validate_params(&params.to_string(), &cache.validators(operation)?.input)?;
        }
        let operation = operation.clone();
        Ok(Self {
            rt,
            p,
            env,
            cache,
            operation,
            client: http::client()?,
        })
    }
    pub(crate) async fn prepare(&self, params: &Value, key: &str) -> Result<Value> {
        prepare_with(
            self.rt,
            &self.p,
            self.env,
            &self.client,
            &self.operation,
            params,
            key,
        )
        .await
    }
    pub(crate) async fn authorize(&self, id: &str, args: &str, contract: &str) -> Result<Value> {
        authorize_with(self.rt, &self.p, self.env, &self.client, id, args, contract).await
    }
    pub(crate) async fn invoke(&self, id: &str, arguments: &str) -> Result<Value> {
        invoke_with(
            self.rt,
            &self.p,
            self.env,
            &self.client,
            &self.cache,
            id,
            Some(arguments),
        )
        .await
    }
    pub(crate) async fn intent(&self, id: &str) -> Result<Value> {
        intent_action_with(
            self.rt,
            &self.p,
            self.env,
            &self.client,
            "intents.status",
            id,
        )
        .await
    }
    pub(crate) async fn submit(&self, params: &Value) -> Result<Value> {
        call_with(
            self.rt,
            &self.p,
            self.env,
            &self.client,
            &self.cache,
            ReadRequest {
                operation: &self.operation,
                params,
                job: true,
            },
        )
        .await
        .map(provenance)
    }
    pub(crate) async fn job(&self, id: &str) -> Result<Value> {
        job_action_with(self.rt, &self.p, self.env, &self.client, "jobs.status", id)
            .await
            .map(provenance)
    }
    pub(crate) async fn wait(&self, id: &str, seconds: u64) -> Result<Value> {
        wait_with(
            self.rt,
            &self.p,
            self.env,
            &self.client,
            id,
            tokio::time::Instant::now() + Duration::from_secs(seconds),
        )
        .await
        .map(provenance)
    }
    pub(crate) fn validate_result(&self, result: &Value) -> Result<()> {
        if !self
            .cache
            .validators(&self.operation)?
            .output
            .is_valid(&result["items"])
        {
            return Err(Failure::new(
                "UPSTREAM_CONTRACT_MISMATCH",
                5,
                "完整任务结果与输出合同不符。",
            ));
        }
        Ok(())
    }
}

fn provenance(mut value: Value) -> Value {
    for (key, meta) in [
        ("operationId", "operationId"),
        ("operationVersion", "contractVersion"),
        ("contractDigest", "contractDigest"),
        ("argumentsDigest", "argumentsDigest"),
    ] {
        value[key] = value["_meta"][meta].clone();
    }
    value
}
