use crate::{
    WorkflowPlan, credentials, http, login,
    output::{Executed, Failure, Result},
    pagination, profile,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{Duration, Instant},
};
pub struct Runtime {
    pub root: PathBuf,
    pub environments: BTreeMap<String, profile::Environment>,
    pub store: Box<dyn credentials::CredentialStore>,
    pub browser: Box<dyn login::Browser>,
    pub interactive: bool,
    /// Absolute budget for one bounded aggregation; covers lock waits, refreshes and retries.
    pub aggregate_budget: Duration,
}
impl Runtime {
    pub fn environment(&self, name: &str) -> Result<&profile::Environment> {
        self.environments.get(name).ok_or_else(|| {
            Failure::new(
                "ENVIRONMENT_NOT_CONFIGURED",
                2,
                "该环境没有经过核实的发行配置。",
            )
        })
    }
    pub(crate) async fn check(&self, name: &str) -> Result<Value> {
        self.online(name, "iam.profile.get", json!({})).await
    }
    /// Re-reads and, when needed, rotates the record under the authorization lock.
    async fn refresh_locked(
        &self,
        p: &profile::Profile,
        env: &profile::Environment,
        client: &reqwest::Client,
        stale: &credentials::Credentials,
    ) -> Result<credentials::Credentials> {
        let _lock = profile::authorization_lock(&self.root, &p.authorization_id).await?;
        let mut c = self
            .store
            .read(&p.credential_key)?
            .ok_or_else(|| Failure::new("AUTH_REQUIRED", 3, "本地没有凭证。"))?;
        let recovering = c.refresh_state == "pending";
        // Another process may have rotated while this caller waited for the lock.
        if c.generation != stale.generation
            && c.access_expires_at
                .as_deref()
                .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
                .is_some_and(|v| v > chrono::Utc::now())
        {
            return Ok(c);
        }
        let request_id = zeroize::Zeroizing::new(if recovering {
            c.refresh_request_id.clone().ok_or_else(|| {
                Failure::new(
                    "AUTH_REFRESH_OUTCOME_UNKNOWN",
                    3,
                    "旧版本的续期结果未知，没有可恢复的请求记录，请重新授权。",
                )
            })?
        } else {
            login::random()
        });
        c.refresh_request_id = Some(request_id.to_string());
        c.refresh_state = "pending".into();
        self.store.write(&p.credential_key, &c)?;
        let token =
            match http::refresh_recoverable(client, env, &c.refresh_token, &request_id, recovering)
                .await
            {
                Ok(token) => token,
                Err(failure) if failure.code == "AUTH_REQUIRED" => {
                    c.refresh_state = "ready".into();
                    c.refresh_request_id = None;
                    self.store.write(&p.credential_key, &c)?;
                    return Err(failure);
                }
                Err(failure) => return Err(failure),
            };
        if token.authorization_id != p.authorization_id
            || token.authorization_expires_at != p.authorization_expires_at
        {
            return Err(Failure::new(
                "IDENTITY_MISMATCH",
                4,
                "续期授权与 profile 不一致。",
            ));
        }
        let expires = token.access_expiry()?;
        c.access_token = token.access_token;
        c.refresh_token = token.refresh_token;
        c.version = 1;
        c.generation = c.generation.checked_add(1).ok_or_else(http::protocol)?;
        c.access_expires_at = Some(expires);
        c.authorization_expires_at = Some(token.authorization_expires_at);
        c.refresh_state = "ready".into();
        c.refresh_request_id = None;
        self.store.write(&p.credential_key, &c)?;
        Ok(c)
    }
    /// Credential acquisition for callers outside this module.
    pub async fn authorize(
        &self,
        p: &profile::Profile,
        env: &profile::Environment,
    ) -> Result<credentials::Credentials> {
        self.credentials(p, env, &http::client()?).await
    }
    /// The single recovery path used by reads after an explicit expiry signal.
    pub async fn recover_credentials(
        &self,
        p: &profile::Profile,
        env: &profile::Environment,
        client: &reqwest::Client,
    ) -> Result<credentials::Credentials> {
        let stale = self
            .store
            .read(&p.credential_key)?
            .ok_or_else(|| Failure::new("AUTH_REQUIRED", 3, "本地没有凭证。"))?;
        self.refresh_locked(p, env, client, &stale).await
    }
    pub(crate) async fn recover_credentials_after(
        &self,
        p: &profile::Profile,
        env: &profile::Environment,
        client: &reqwest::Client,
        stale: &credentials::Credentials,
    ) -> Result<credentials::Credentials> {
        self.refresh_locked(p, env, client, stale).await
    }
    pub(crate) async fn credentials(
        &self,
        p: &profile::Profile,
        env: &profile::Environment,
        client: &reqwest::Client,
    ) -> Result<credentials::Credentials> {
        let c = self
            .store
            .read(&p.credential_key)?
            .ok_or_else(|| Failure::new("AUTH_REQUIRED", 3, "本地没有凭证。"))?;
        let expired = c
            .access_expires_at
            .as_deref()
            .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
            .is_some_and(|v| v <= chrono::Utc::now());
        if expired || c.refresh_state == "pending" {
            return self.refresh_locked(p, env, client, &c).await;
        }
        Ok(c)
    }
    pub(crate) async fn online(&self, name: &str, id: &str, params: Value) -> Result<Value> {
        let p = profile::read(&self.root, name)?
            .ok_or_else(|| Failure::new("PROFILE_NOT_CONFIGURED", 2, "profile 尚未配置。"))?;
        let env = self.environment(&p.environment)?;
        let client = http::client()?;
        let mut c = self.credentials(&p, env, &client).await?;
        let call = if id == "iam.profile.get" {
            http::me(&client, env, &c).await
        } else {
            http::operation(&client, env, &c, id, params.clone()).await
        };
        let value = match call {
            Err(e) if e.code == "ACCESS_TOKEN_EXPIRED" => {
                c = self.recover_credentials_after(&p, env, &client, &c).await?;
                if id == "iam.profile.get" {
                    http::me(&client, env, &c).await?
                } else {
                    http::operation(&client, env, &c, id, params).await?
                }
            }
            other => other?,
        };
        if id == "iam.profile.get"
            && (value["subject"]["id"] != p.subject_id
                || value["authorizationId"] != p.authorization_id)
        {
            return Err(Failure::new(
                "IDENTITY_MISMATCH",
                4,
                "在线身份与本地绑定不一致。",
            ));
        }
        Ok(value)
    }
    /// One page request through the shared executor, including the single resend budget
    /// and the per-page contract checks.
    pub(crate) async fn online_page(
        &self,
        name: &str,
        page: i64,
        page_size: i64,
        params: Value,
    ) -> Result<Value> {
        let p = profile::read(&self.root, name)?
            .ok_or_else(|| Failure::new("PROFILE_NOT_CONFIGURED", 2, "profile 尚未配置。"))?;
        let env = self.environment(&p.environment)?;
        let client = http::client()?;
        let mut c = self.credentials(&p, env, &client).await?;
        let deadline = Instant::now() + self.aggregate_budget;
        let mut cancel = pagination::Cancel::disabled();
        match pagination::fetch_page(
            self,
            &mut cancel,
            deadline,
            &client,
            &p,
            env,
            &mut c,
            &params,
            // fetch_page enforces 2 MiB per response; retries share the total byte budget.
            http::AGGREGATE_BYTE_LIMIT,
        )
        .await
        {
            pagination::Attempt::Stopped(stop) => Err(stop),
            pagination::Attempt::Page(mut response) => {
                if let Some(failure) = response.failure.take() {
                    if failure.code == "PAGINATION_UNKNOWN" {
                        let stage = if response.status == 200 && response.pagination.is_none() {
                            "deserialize"
                        } else {
                            "server_failure"
                        };
                        let diagnostic = http::pagination_diagnostic(
                            &response.raw_meta,
                            stage,
                            page,
                            page_size,
                            response.data.len(),
                        );
                        let mut failure = failure;
                        failure.trace_id = response.trace_id.clone();
                        return Err(failure.with_partial(
                            Value::Null,
                            json!({
                                "traceId": response.trace_id,
                                "paginationDiagnostic": diagnostic,
                            }),
                        ));
                    }
                    return Err(failure);
                }
                match response.pagination.as_ref() {
                    Some(pagination) => {
                        if let Some(problem) =
                            pagination::page_problem(pagination, page, page_size, &response, false)
                        {
                            if problem.pagination {
                                let diagnostic = http::pagination_diagnostic(
                                    &response.raw_meta,
                                    "contract",
                                    page,
                                    page_size,
                                    response.data.len(),
                                );
                                let mut failure = http::pagination_unknown();
                                failure.trace_id = response.trace_id.clone();
                                return Err(failure.with_partial(
                                    Value::Null,
                                    json!({
                                        "traceId": response.trace_id,
                                        "paginationDiagnostic": diagnostic,
                                    }),
                                ));
                            }
                            return Err(http::protocol());
                        }
                    }
                    None => {
                        let diagnostic = http::pagination_diagnostic(
                            &response.raw_meta,
                            "missing",
                            page,
                            page_size,
                            response.data.len(),
                        );
                        let mut failure = http::pagination_unknown();
                        failure.trace_id = response.trace_id.clone();
                        return Err(failure.with_partial(
                            Value::Null,
                            json!({
                                "traceId": response.trace_id,
                                "paginationDiagnostic": diagnostic,
                            }),
                        ));
                    }
                }
                // The page envelope keeps the server meta verbatim, as in the single-page contract.
                Ok(json!({"items":response.data,"_meta":response.raw_meta}))
            }
        }
    }
    /// The bounded aggregation shared by the dedicated command and `api call`.
    pub(crate) async fn online_all(
        &self,
        name: &str,
        plan: &WorkflowPlan,
        params: &Value,
    ) -> Result<Executed> {
        let started = Instant::now();
        let deadline = started + self.aggregate_budget;
        let mut cancel = pagination::Cancel::signal();
        let p = profile::read(&self.root, name)?
            .ok_or_else(|| Failure::new("PROFILE_NOT_CONFIGURED", 2, "profile 尚未配置。"))?;
        let env = self.environment(&p.environment)?;
        let client = http::client()?;
        let mut c = match cancel
            .race(deadline, self.credentials(&p, env, &client))
            .await
        {
            Ok(Ok(c)) => c,
            Ok(Err(failure)) => return Ok(Executed::failed(failure)),
            Err(stop) => return Ok(Executed::failed(stop)),
        };
        let result = pagination::aggregate(
            self,
            &mut cancel,
            deadline,
            &client,
            &p,
            env,
            &mut c,
            params,
            plan.limits,
        )
        .await;
        Ok(result)
    }
    pub(crate) async fn logout(&self, name: &str) -> Result<Value> {
        let p = match profile::read(&self.root, name)? {
            Some(p) => p,
            None => {
                return Ok(
                    json!({"remoteRevocation":"not-locatable","localCleanup":"not-present"}),
                );
            }
        };
        let env = self.environment(&p.environment)?;
        let _lock = profile::authorization_lock(&self.root, &p.authorization_id).await?;
        let c = self.store.read(&p.credential_key)?;
        if c.is_none() {
            return Ok(
                json!({"authorizationId":p.authorization_id,"remoteRevocation":"not-locatable","localCleanup":"not-present","recoveryUrl":env.recovery_url}),
            );
        }
        let remote = http::revoke(&http::client()?, env, &c.as_ref().unwrap().refresh_token).await;
        self.store.delete(&p.credential_key)?;
        if !remote {
            let mut e = Failure::new("LOGOUT_PARTIAL", 7, "本地凭证已清理，远端撤销结果未知。");
            e.recovery = Some(Box::new(
                json!({"authorizationId":p.authorization_id,"remoteRevocation":"unknown","localCleanup":"confirmed","recoveryUrl":env.recovery_url}),
            ));
            return Err(e);
        }
        Ok(
            json!({"authorizationId":p.authorization_id,"remoteRevocation":"confirmed","localCleanup":"confirmed","recoveryUrl":env.recovery_url}),
        )
    }
}
