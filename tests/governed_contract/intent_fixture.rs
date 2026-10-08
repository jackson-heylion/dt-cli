use super::*;

impl Fake {
    pub(super) fn intent_failure(&self, status: u16, code: &str, id: &str) -> Reply {
        let view = &self.intents[id];
        reply(
            status,
            json!({"schemaVersion":"1.0","operationId":view["operationId"],"ok":false,"profile":null,
                "data":null,"error":{"code":code,"category":"contract","message":"写入原文不外泄",
                    "retryable":false,"hint":"none"},
                "meta":{"traceId":"trace-intent","state":view["state"],"sideEffect":view["sideEffect"],
                    "recovery":view}}),
        )
    }
    pub(super) fn intent(
        &mut self,
        request: &Seen,
        path: &str,
        label: &str,
        bearer: &str,
    ) -> Reply {
        match label {
            "intents.prepare" => {
                if !self.system_tokens.iter().any(|t| t == bearer) {
                    return failure(401, "AUTH_REQUIRED");
                }
                let body: Value = serde_json::from_str(&request.body).unwrap();
                let key = request.idempotency.clone().unwrap_or_default();
                let arguments = body["arguments"].to_string();
                if let Some((id, known)) = self.intent_keys.get(&key).cloned() {
                    if known != arguments {
                        return failure(409, "IDEMPOTENCY_CONFLICT");
                    }
                    return reply(201, ok(LIKE, self.intents[&id].clone()));
                }
                self.counter += 1;
                let id = format!("{:I>43}", self.counter);
                let mut view = json!({"intentId":id,"state":"prepared","sideEffect":"none","reason":null,
                    "operationId":body["contract"]["operationId"],
                    "operationVersion":body["contract"]["operationVersion"],
                    "contractDigest":body["contract"]["contractDigest"],
                    "argumentsDigest":digest(&arguments),"expiresAt":"2030-01-01T00:05:00Z",
                    "downstreamRecordId":null,
                    "confirmationUrl":null});
                if self.auto_approve_prepared {
                    view["state"] = json!("approved");
                }
                self.intent_keys.insert(key, (id.clone(), arguments));
                self.intents.insert(id, view.clone());
                if let Some(path) = self.params_mutation.take() {
                    std::fs::write(path, r#"{"specificDeeds":"changed"}"#).unwrap();
                }
                self.break_record("intents.prepare");
                if self.hold_prepare_reply_once {
                    self.hold_prepare_reply_once = false;
                    return (1, Vec::new(), Vec::new());
                }
                if self.drop_prepare_reply_once {
                    self.drop_prepare_reply_once = false;
                    return (0, Vec::new(), Vec::new());
                }
                reply(201, ok(LIKE, view))
            }
            _ => {
                if bearer != self.access || self.revoked {
                    return failure(401, "AUTH_REQUIRED");
                }
                let id = path.trim_start_matches("/cli-api/v1/intents/");
                let id = id.split('/').next().unwrap_or_default().to_owned();
                let Some(state) = self.intents.get(&id).map(|v| v["state"].clone()) else {
                    return failure(404, "RESOURCE_UNAVAILABLE");
                };
                if label == "intents.authorize" {
                    let body: Value = serde_json::from_str(&request.body).unwrap();
                    let view = &self.intents[&id];
                    if body["argumentsDigest"] != view["argumentsDigest"]
                        || body["contractDigest"] != view["contractDigest"]
                    {
                        return failure(409, "CONTRACT_CHANGED");
                    }
                    if state != "prepared" && state != "approved" {
                        return self.intent_failure(409, "ALREADY_DISPATCHED", &id);
                    }
                    self.set_intent(&id, "approved", "none");
                    self.break_record("intents.authorize");
                    if self.hold_authorize_reply_once {
                        self.hold_authorize_reply_once = false;
                        return (1, Vec::new(), Vec::new());
                    }
                    if self.drop_authorize_reply_once {
                        self.drop_authorize_reply_once = false;
                        return (0, Vec::new(), Vec::new());
                    }
                }
                if label == "intents.cancel" {
                    if state == "prepared" || state == "approved" {
                        self.set_intent(&id, "cancelled", "none");
                    } else {
                        return self.intent_failure(409, "ALREADY_DISPATCHED", &id);
                    }
                }
                reply(200, ok(LIKE, self.intents[&id].clone()))
            }
        }
    }
    fn break_record(&mut self, label: &str) {
        if self
            .record_fault
            .as_ref()
            .is_some_and(|(phase, _)| *phase == label)
        {
            let (_, root) = self.record_fault.take().unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                for entry in std::fs::read_dir(root).unwrap() {
                    let path = entry.unwrap().path();
                    if path.extension().is_some_and(|e| e == "json") {
                        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666))
                            .unwrap();
                    }
                }
            }
        }
    }
    pub(super) fn set_intent(&mut self, id: &str, state: &str, side_effect: &str) {
        let view = self.intents.get_mut(id).unwrap();
        view["state"] = json!(state);
        view["sideEffect"] = json!(side_effect);
        if state != "prepared" && state != "unknown" {
            view["confirmationUrl"] = Value::Null;
        }
    }
    /// IAM's single dispatch of an intent: only an approved intent is sent, and only once.
    pub(super) fn dispatch_intent(&mut self, id: &str) -> Reply {
        let Some(view) = self.intents.get(id).cloned() else {
            return failure(404, "RESOURCE_UNAVAILABLE");
        };
        match view["state"].as_str().unwrap() {
            "approved" => {
                self.writes += 1;
                match self.dispatch.take() {
                    Some("unknown") => {
                        self.set_intent(id, "unknown", "unknown");
                        return self.intent_failure(202, "SIDE_EFFECT_UNKNOWN", id);
                    }
                    Some("drop") => {
                        self.set_intent(id, "unknown", "unknown");
                        return (0, Vec::new(), Vec::new());
                    }
                    Some("hold") => {
                        self.set_intent(id, "dispatching", "unknown");
                        return (1, Vec::new(), Vec::new());
                    }
                    _ => self.set_intent(id, "succeeded", "confirmed"),
                }
            }
            "succeeded" => {}
            "prepared" => return self.intent_failure(409, "APPROVAL_REQUIRED", id),
            "unknown" | "dispatching" => {
                return self.intent_failure(202, "SIDE_EFFECT_UNKNOWN", id);
            }
            _ => return self.intent_failure(409, "INTENT_TERMINAL", id),
        }
        self.break_record("systems.invoke");
        reply(
            200,
            json!({"schemaVersion":"1.0","operationId":view["operationId"],"ok":true,"profile":null,
                "data":{"intentId":id,"state":"succeeded","sideEffect":"confirmed","downstreamRecordId":null},
                "error":null,
                "meta":{"contractVersion":view["operationVersion"],"contractDigest":view["contractDigest"],
                    "traceId":"trace-dispatch","invocationId":"inv-9","state":"succeeded",
                    "sideEffect":"confirmed","asOf":"2026-09-24T08:01:00Z","complete":null,
                    "nextPage":null,"itemsReturned":null,"recovery":null}}),
        )
    }
}
