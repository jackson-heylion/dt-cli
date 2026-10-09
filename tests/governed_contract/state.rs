use super::*;

impl Fake {
    pub(super) fn new(system: &str, environment: &str, operations: Vec<Value>) -> Self {
        Self {
            seen: Vec::new(),
            system: system.into(),
            environment: environment.into(),
            subject: "1".into(),
            authorization: String::new(),
            authorization_expires_at: (chrono::Utc::now() + chrono::Duration::days(7))
                .format("%Y-%m-%dT%H:%M:%SZ")
                .to_string(),
            authorize: Vec::new(),
            challenge: None,
            code: String::new(),
            access: String::new(),
            refresh: String::new(),
            retired_refresh: Vec::new(),
            refresh_receipts: BTreeMap::new(),
            drop_refresh_reply_once: false,
            system_tokens: Vec::new(),
            issued: Vec::new(),
            counter: 0,
            revoked: false,
            catalog_401_once: false,
            operations,
            items: items(),
            jobs: BTreeMap::new(),
            job_contracts: BTreeMap::new(),
            auto_complete_jobs: false,
            job_initial_state: None,
            overrides: BTreeMap::new(),
            rate_limit_once: None,
            origin: String::new(),
            intents: BTreeMap::new(),
            intent_keys: BTreeMap::new(),
            writes: 0,
            dispatch: None,
            auto_approve_prepared: false,
            hold_prepare_reply_once: false,
            hold_authorize_reply_once: false,
            drop_prepare_reply_once: false,
            drop_authorize_reply_once: false,
            params_mutation: None,
            record_fault: None,
        }
    }
    pub(super) fn mint(&mut self, prefix: &str) -> String {
        self.counter += 1;
        let token = format!("{prefix}{:A>43}", format!("CANARY{}", self.counter));
        self.issued.push(token.clone());
        token
    }
    pub(super) fn token_body(&self) -> Value {
        json!({"access_token":self.access,"refresh_token":self.refresh,"token_type":"Bearer",
            "expires_in":43200,"authorizationId":self.authorization,
            "authorizationExpiresAt":self.authorization_expires_at,
            "systemId":self.system,"environment":self.environment})
    }
    pub(super) fn catalog(&self) -> Value {
        ok(
            "catalog",
            json!({"systemId":self.system,"environment":self.environment,
                "authorizationId":self.authorization,
                "authorizationExpiresAt":self.authorization_expires_at,
                "subject":{"id":self.subject,"account":"employee","displayName":"Employee"},
                "cachedAt":"2026-09-24T08:00:00Z","catalogRevision":"7",
                "operations":self.operations}),
        )
    }
    pub(super) fn contract(&self, key: &str) -> Option<&Value> {
        self.operations.iter().find(|op| {
            op["consented"] == true
                && format!(
                    "{}@{}#{}",
                    op["operationId"].as_str().unwrap(),
                    op["operationVersion"].as_str().unwrap(),
                    op["contractDigest"].as_str().unwrap()
                ) == key
        })
    }
    pub(super) fn handle(&mut self, request: &Seen) -> Reply {
        let path = request
            .path
            .split('?')
            .next()
            .unwrap_or_default()
            .to_owned();
        let label = match (request.method.as_str(), path.as_str()) {
            ("POST", "/cli-api/oauth2/token") => "token",
            ("POST", "/cli-api/oauth2/exchange") => "exchange",
            ("POST", "/cli-api/oauth2/revoke") => "revoke",
            ("GET", "/cli-api/v1/catalog") => "catalog",
            ("POST", "/cli-api/v1/jobs") => "jobs.submit",
            ("POST", "/cli-api/v1/intents") => "intents.prepare",
            ("POST", p) if p.starts_with("/cli-api/v1/intents/") && p.ends_with("/cancel") => {
                "intents.cancel"
            }
            ("POST", p) if p.starts_with("/cli-api/v1/intents/") && p.ends_with("/authorize") => {
                "intents.authorize"
            }
            ("GET", p) if p.starts_with("/cli-api/v1/intents/") => "intents.status",
            ("POST", p) if p.starts_with("/cli-api/v1/systems/") && p.ends_with("/invoke") => {
                "invoke"
            }
            ("GET", p) if p.starts_with("/cli-api/v1/jobs/") && p.ends_with("/result") => {
                "jobs.result"
            }
            ("POST", p) if p.starts_with("/cli-api/v1/jobs/") && p.ends_with("/cancel") => {
                "jobs.cancel"
            }
            ("GET", p) if p.starts_with("/cli-api/v1/jobs/") => "jobs.status",
            ("GET", "/cli/v1/me") => "me",
            _ => "unknown",
        };
        if self.rate_limit_once == Some(label) {
            self.rate_limit_once = None;
            return (
                429,
                vec![("Retry-After".into(), "1".into())],
                serde_json::to_vec(&json!({"error":{"code":"RATE_LIMITED"}})).unwrap(),
            );
        }
        if let Some(reply) = self.overrides.get(label) {
            return reply.clone();
        }
        let bearer = request
            .authorization
            .as_deref()
            .and_then(|v| v.strip_prefix("Bearer "))
            .unwrap_or_default()
            .to_owned();
        if label.starts_with("intents.") {
            return self.intent(request, &path, label, &bearer);
        }
        match label {
            "token" => {
                let form = request.form();
                match form.get("grant_type").map(String::as_str) {
                    Some("authorization_code") => {
                        let verifier = form.get("code_verifier").cloned().unwrap_or_default();
                        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
                        if self.code.is_empty()
                            || form.get("code") != Some(&self.code)
                            || Some(challenge) != self.challenge
                            || form.get("client_id").map(String::as_str) != Some("dt-cli")
                            || !form.contains_key("redirect_uri")
                        {
                            return oauth_error(400, "invalid_grant", "AUTH_DENIED");
                        }
                        self.code.clear();
                        self.counter += 1;
                        self.authorization = format!("authz-{}", self.counter);
                        self.revoked = false;
                        self.retired_refresh.clear();
                        self.refresh_receipts.clear();
                        self.access = self.mint("dtcli_g_a_");
                        self.refresh = self.mint("dtcli_g_r_");
                        reply(200, self.token_body())
                    }
                    Some("refresh_token") => {
                        let presented = form.get("refresh_token").cloned().unwrap_or_default();
                        if self.revoked {
                            return oauth_error(403, "invalid_grant", "AUTHORIZATION_REVOKED");
                        }
                        if presented == self.refresh {
                            self.retired_refresh.push(presented.clone());
                            self.access = self.mint("dtcli_g_a_");
                            self.refresh = self.mint("dtcli_g_r_");
                            let body = self.token_body();
                            if let Some(id) = form.get("request_id") {
                                self.refresh_receipts.insert(
                                    presented,
                                    (id.clone(), body.clone(), self.refresh.clone()),
                                );
                            }
                            if std::mem::take(&mut self.drop_refresh_reply_once) {
                                (0, Vec::new(), Vec::new())
                            } else {
                                reply(200, body)
                            }
                        } else if self.retired_refresh.contains(&presented) {
                            if let Some((id, body, successor)) =
                                self.refresh_receipts.get(&presented)
                                && Some(id) == form.get("request_id")
                                && successor == &self.refresh
                            {
                                return reply(200, body.clone());
                            }
                            // A different recovery request is a replay of the retired token.
                            self.revoked = true;
                            oauth_error(403, "invalid_grant", "AUTHORIZATION_REVOKED")
                        } else {
                            oauth_error(400, "invalid_grant", "AUTH_REQUIRED")
                        }
                    }
                    _ => oauth_error(400, "invalid_request", "INVALID_ARGUMENT"),
                }
            }
            "revoke" => {
                let token = request.form().get("token").cloned().unwrap_or_default();
                if token == self.refresh || token == self.access {
                    self.revoked = true;
                }
                (200, Vec::new(), Vec::new())
            }
            "catalog" => {
                if self.revoked {
                    return failure(403, "AUTHORIZATION_REVOKED");
                }
                if bearer != self.access || std::mem::take(&mut self.catalog_401_once) {
                    return failure(401, "AUTH_REQUIRED");
                }
                reply(200, self.catalog())
            }
            "exchange" => {
                let form = request.form();
                if self.revoked {
                    return oauth_error(403, "invalid_grant", "AUTHORIZATION_REVOKED");
                }
                if form.get("subject_token") != Some(&self.access)
                    || form.get("audience") != Some(&self.system)
                {
                    return oauth_error(403, "invalid_grant", "SCOPE_DENIED");
                }
                let contracts: Vec<String> = serde_json::from_str(
                    form.get("operation_contracts")
                        .map(String::as_str)
                        .unwrap_or("[]"),
                )
                .unwrap_or_default();
                if contracts.is_empty() || contracts.iter().any(|key| self.contract(key).is_none())
                {
                    let changed = contracts.iter().any(|requested| {
                        self.operations.iter().any(|op| {
                            requested.split('#').next() == key(op).split('#').next()
                                && requested != &key(op)
                        })
                    });
                    return if changed {
                        oauth_error(409, "invalid_target", "CONTRACT_CHANGED")
                    } else {
                        oauth_error(403, "invalid_scope", "SCOPE_DENIED")
                    };
                }
                let token = self.mint("dtcli_g_s_");
                self.system_tokens.push(token.clone());
                reply(
                    200,
                    json!({"access_token":token,"token_type":"Bearer","expires_in":300,
                        "audience":self.system,"operation_contracts":contracts}),
                )
            }
            "invoke" => {
                if !self.system_tokens.contains(&bearer) {
                    return failure(401, "AUTH_REQUIRED");
                }
                let body: Value = serde_json::from_str(&request.body).unwrap();
                if let Some(id) = body["intentId"].as_str() {
                    return self.dispatch_intent(id);
                }
                let contract = &body["contract"];
                reply(
                    200,
                    json!({"schemaVersion":"1.0","operationId":contract["operationId"],"ok":true,
                        "profile":null,"data":self.items,"error":null,
                        "meta":{"contractVersion":contract["operationVersion"],
                            "contractDigest":contract["contractDigest"],"traceId":"trace-invoke",
                            "invocationId":"inv-1","state":"succeeded","sideEffect":"none",
                            "asOf":"2026-09-24T08:00:00Z","complete":true,"nextPage":null,
                            "itemsReturned":1,"recovery":null,
                            "upstreamHost":"supply.internal.invalid"}}),
                )
            }
            "jobs.submit" => {
                if !self.system_tokens.contains(&bearer) {
                    return failure(401, "AUTH_REQUIRED");
                }
                self.counter += 1;
                let id = format!("{:J>43}", self.counter);
                self.jobs.insert(
                    id.clone(),
                    self.job_initial_state
                        .unwrap_or(if self.auto_complete_jobs {
                            "succeeded"
                        } else {
                            "queued"
                        })
                        .into(),
                );
                let request: Value = serde_json::from_str(&request.body).unwrap();
                self.job_contracts.insert(id.clone(),json!({"operationId":request["contract"]["operationId"],
                    "contractVersion":request["contract"]["operationVersion"],"contractDigest":request["contract"]["contractDigest"],
                    "argumentsDigest":digest(&request["arguments"].to_string()),"traceId":"trace-job"}));
                let mut body = ok(
                    "job.submit",
                    json!({"jobId":id,"state":"queued","expiresAt":"2026-09-24T08:10:00Z"}),
                );
                body["meta"] = self.job_contracts[&id].clone();
                reply(202, body)
            }
            "jobs.status" | "jobs.result" | "jobs.cancel" => {
                if bearer != self.access || self.revoked {
                    return failure(401, "AUTH_REQUIRED");
                }
                let id = path.trim_start_matches("/cli-api/v1/jobs/");
                let id = id.split('/').next().unwrap_or_default().to_owned();
                let Some(state) = self.jobs.get(&id).cloned() else {
                    return failure(404, "RESOURCE_UNAVAILABLE");
                };
                let response = match label {
                    "jobs.status" => reply(
                        200,
                        ok(
                            "job.status",
                            json!({"jobId":id,"state":state,"itemCount":0,"pageCount":0,
                                "complete":state == "succeeded","failureCode":null}),
                        ),
                    ),
                    "jobs.result" if state == "succeeded" => reply(
                        200,
                        ok(
                            "job.result",
                            json!({"jobId":id,"state":"succeeded","complete":true,
                                "scope":"current-authorized-shops","items":self.items,
                                "itemCount":1,"pageCount":3}),
                        ),
                    ),
                    "jobs.result" => failure(409, "PARTIAL_RESULT"),
                    _ if state == "queued" || state == "running" => {
                        self.jobs.insert(id.clone(), "cancelled".into());
                        reply(
                            200,
                            ok(
                                "job.cancel",
                                json!({"jobId":id,"state":"cancelled","complete":false}),
                            ),
                        )
                    }
                    _ => failure(409, "INTENT_TERMINAL"),
                };
                let mut body: Value = serde_json::from_slice(&response.2).unwrap();
                if body["ok"] == true
                    && let Some(meta) = self.job_contracts.get(&id)
                {
                    body["meta"] = meta.clone();
                }
                reply(response.0, body)
            }
            "me" => {
                if bearer != OLD_ACCESS {
                    return failure(401, "AUTH_REQUIRED");
                }
                reply(
                    200,
                    json!({"schemaVersion":"1.0","operationId":"iam.profile.get","ok":true,
                        "profile":null,"error":null,"meta":{"traceId":"0123456789abcdef"},
                        "data":{"subject":{"id":"1","account":"employee","displayName":"Employee"},
                            "authorizationId":"auth-fixture",
                            "authorizationExpiresAt":self.authorization_expires_at,
                            "allowedOperations":["iam.profile.get"],
                            "checkedAt":"2026-09-24T08:00:00Z"}}),
                )
            }
            _ => failure(404, "RESOURCE_UNAVAILABLE"),
        }
    }
}
