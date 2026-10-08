use super::*;

struct State {
    page: i64,
    pages_read: usize,
    bytes: usize,
    items_fetched: usize,
    kept: Vec<Value>,
    seen: HashSet<String>,
    matched_total: Option<i64>,
    has_more: Option<bool>,
    next_page: Option<i64>,
    page_complete: Option<bool>,
    scope_complete: Option<bool>,
    truncated: bool,
    reason: Option<&'static str>,
    content_complete: bool,
    synced: Option<String>,
    synced_conflict: bool,
    trace_id: Option<String>,
    pagination_diagnostic: Option<Value>,
}
impl State {
    fn new() -> Self {
        Self {
            page: 1,
            pages_read: 0,
            bytes: 0,
            items_fetched: 0,
            kept: Vec::new(),
            seen: HashSet::new(),
            matched_total: None,
            has_more: None,
            next_page: None,
            page_complete: None,
            scope_complete: None,
            truncated: false,
            reason: None,
            content_complete: true,
            synced: None,
            synced_conflict: false,
            trace_id: None,
            pagination_diagnostic: None,
        }
    }
    fn meta(&self, page_size: i64) -> Value {
        let mut meta = json!({
            "traceId": self.trace_id,
            "budget": {"bytesRead": self.bytes},
            "pagination": {
                "mode": "all",
                "page": self.page,
                "pageSize": page_size,
                "pagesRead": self.pages_read,
                "matchedTotal": self.matched_total,
                "itemsFetched": self.items_fetched,
                "itemsReturned": self.kept.len(),
                "hasMore": self.has_more,
                "nextPage": self.next_page,
                "pageComplete": self.page_complete,
                "scopeComplete": self.scope_complete,
                "truncated": self.truncated,
                "reason": self.reason,
                "contentComplete": self.content_complete,
            },
            "source": {
                "fetchedAt": chrono::Utc::now().to_rfc3339(),
                "sourceSyncedAt": if self.synced_conflict { Value::Null } else { json!(self.synced) },
                "freshness": "unknown",
                "consistency": "non_snapshot",
            },
        });
        if let Some(diagnostic) = &self.pagination_diagnostic {
            meta["paginationDiagnostic"] = diagnostic.clone();
        }
        meta
    }
    fn set_pagination_diagnostic(&mut self, response: &PageResponse, stage: &str, page_size: i64) {
        self.pagination_diagnostic = Some(http::pagination_diagnostic(
            &response.raw_meta,
            stage,
            self.page,
            page_size,
            response.data.len(),
        ));
    }
    fn records(&mut self) -> Value {
        Value::Array(std::mem::take(&mut self.kept))
    }
    /// Pages already read stay authoritative; the stop reason never rewrites them.
    fn stop(&mut self, reason: &'static str, truncated: bool, scope: Option<bool>) {
        self.truncated = truncated;
        if self.scope_complete.is_none() {
            self.scope_complete = scope;
        }
        if self.reason.is_none() {
            self.reason = Some(reason);
        }
    }
    fn finish(&mut self, failure: Failure, page_size: i64) -> Executed {
        if let Some(trace) = failure.trace_id.clone() {
            self.trace_id = Some(trace);
        }
        let meta = self.meta(page_size);
        if self.pages_read == 0 {
            // A first-page failure is not a partial result: nothing was retrieved.
            return Executed::failed(failure.with_partial(Value::Null, meta));
        }
        Executed::failed(failure.with_partial(self.records(), meta))
    }
}

/// Reads at most `limits` pages and records under one absolute deadline.
#[allow(clippy::too_many_arguments)]
pub async fn aggregate(
    rt: &Runtime,
    cancel: &mut Cancel,
    deadline: Instant,
    client: &reqwest::Client,
    profile: &Profile,
    env: &Environment,
    credentials: &mut Credentials,
    params: &Value,
    limits: Limits,
) -> Executed {
    aggregate_bounded(
        rt,
        cancel,
        deadline,
        client,
        profile,
        env,
        credentials,
        params,
        Bounds {
            limits,
            bytes: http::AGGREGATE_BYTE_LIMIT,
        },
    )
    .await
}

#[derive(Clone, Copy)]
pub(crate) struct Bounds {
    pub limits: Limits,
    pub bytes: usize,
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn aggregate_bounded(
    rt: &Runtime,
    cancel: &mut Cancel,
    deadline: Instant,
    client: &reqwest::Client,
    profile: &Profile,
    env: &Environment,
    credentials: &mut Credentials,
    params: &Value,
    bounds: Bounds,
) -> Executed {
    let limits = bounds.limits;
    let mut state = State::new();
    loop {
        let remaining = bounds.bytes.saturating_sub(state.bytes);
        if remaining == 0 {
            state.stop(REASON_BYTES, true, Some(false));
            return state.finish(
                limit_reached("已达到本次汇总的字节上限；仍有后续数据未读取。"),
                limits.page_size,
            );
        }
        let allowance = remaining;
        let mut request = params.clone();
        request["page"] = json!(state.page);
        request["pageSize"] = json!(limits.page_size);
        let mut response = match fetch_page(
            rt,
            cancel,
            deadline,
            client,
            profile,
            env,
            credentials,
            &request,
            allowance,
        )
        .await
        {
            Attempt::Page(response) => response,
            Attempt::Stopped(stop) => {
                let cancelled = stop.exit == 8;
                state.stop(
                    if cancelled {
                        REASON_CANCELLED
                    } else {
                        REASON_DEADLINE
                    },
                    !cancelled,
                    Some(false),
                );
                // With pages already read the stop is a partial result, not the original error.
                let failure = if state.pages_read == 0 {
                    stop
                } else if cancelled {
                    partial()
                } else {
                    limit_reached("已达到本次汇总的时间上限；已保留此前完整页面。")
                };
                return state.finish(failure, limits.page_size);
            }
        };
        if let Some(failure) = response.failure.take() {
            state.page_complete = Some(false);
            if response.content_truncated {
                state.content_complete = false;
            }
            if auth_class(failure.code) {
                state.stop(REASON_PARTIAL, true, Some(false));
                return state.finish(failure, limits.page_size);
            }
            if failure.code == "RESULT_LIMIT" {
                state.stop(REASON_BYTES, true, Some(false));
                return state.finish(
                    limit_reached("当前页未能在此次汇总的字节上限内取得；已保留此前完整页面。"),
                    limits.page_size,
                );
            }
            if failure.code == "PAGINATION_UNKNOWN" {
                let stage = if response.status == 200 && response.pagination.is_none() {
                    "deserialize"
                } else {
                    "server_failure"
                };
                state.set_pagination_diagnostic(&response, stage, limits.page_size);
                state.stop(REASON_PAGINATION, false, None);
                return state.finish(failure, limits.page_size);
            }
            if state.pages_read == 0 {
                // Nothing was retrieved: keep the original classification and claim no cause.
                state.stop(REASON_PARTIAL, false, Some(false));
                state.reason = None;
                return state.finish(failure, limits.page_size);
            }
            state.stop(REASON_PARTIAL, true, Some(false));
            let mut decisive = partial();
            decisive.trace_id = failure.trace_id;
            return state.finish(decisive, limits.page_size);
        }
        let Some(pagination) = response.pagination.clone() else {
            state.page_complete = Some(false);
            state.content_complete = false;
            state.set_pagination_diagnostic(&response, "missing", limits.page_size);
            state.stop(REASON_PAGINATION, false, None);
            return state.finish(http::pagination_unknown(), limits.page_size);
        };
        if let Some(problem) = page_problem(
            &pagination,
            state.page,
            limits.page_size,
            &response,
            state.pages_read > 0,
        ) {
            state.page_complete = Some(false);
            if problem.pagination {
                state.set_pagination_diagnostic(&response, "contract", limits.page_size);
            }
            if problem.content {
                state.content_complete = false;
            }
            state.stop(
                if problem.pagination {
                    REASON_PAGINATION
                } else {
                    REASON_PARTIAL
                },
                false,
                if problem.pagination {
                    None
                } else {
                    Some(false)
                },
            );
            return state.finish(
                if problem.pagination {
                    http::pagination_unknown()
                } else {
                    http::protocol()
                },
                limits.page_size,
            );
        }
        // The page is complete and validated: it now counts towards every counter.
        state.pages_read += 1;
        state.bytes += response.bytes;
        state.items_fetched += response.data.len();
        state.page_complete = Some(true);
        if response.trace_id.is_some() {
            state.trace_id = response.trace_id.clone();
        }
        let mut changed_total = false;
        if state.pages_read == 1 {
            state.matched_total = pagination.matched_total;
            state.synced = response
                .source
                .as_ref()
                .and_then(|s| s.source_synced_at.clone());
        } else {
            if state.matched_total.is_some()
                && pagination.matched_total.is_some()
                && state.matched_total != pagination.matched_total
            {
                changed_total = true;
            }
            match response
                .source
                .as_ref()
                .and_then(|s| s.source_synced_at.clone())
            {
                Some(synced) if state.synced.as_deref() == Some(synced.as_str()) => {}
                _ => state.synced_conflict = true,
            }
        }
        state.has_more = pagination.has_more;
        state.next_page = pagination.next_page;
        let mut overflow = false;
        for item in &response.data {
            let id = item["id"].as_str().unwrap_or_default().to_owned();
            if state.seen.contains(&id) {
                // Keep the first occurrence, drop the repeat, stop reading.
                state.stop(REASON_DATA_CHANGED, false, Some(false));
                return state.finish(data_changed(), limits.page_size);
            }
            if state.kept.len() == limits.max_items {
                overflow = true;
                break;
            }
            state.seen.insert(id);
            state.kept.push(item.clone());
        }
        if changed_total {
            state.stop(REASON_DATA_CHANGED, false, Some(false));
            return state.finish(data_changed(), limits.page_size);
        }
        if overflow {
            state.stop(REASON_MAX_ITEMS, true, Some(false));
            return state.finish(
                limit_reached("已达到本次汇总的保留记录上限；仍有数据未读取。"),
                limits.page_size,
            );
        }
        if state
            .matched_total
            .is_some_and(|t| t < state.kept.len() as i64)
        {
            state.set_pagination_diagnostic(&response, "aggregate_consistency", limits.page_size);
            state.stop(REASON_PAGINATION, false, None);
            return state.finish(http::pagination_unknown(), limits.page_size);
        }
        if state
            .matched_total
            .is_some_and(|t| pagination.has_more == Some(true) && state.kept.len() as i64 >= t)
        {
            state.set_pagination_diagnostic(&response, "aggregate_consistency", limits.page_size);
            state.stop(REASON_PAGINATION, false, None);
            return state.finish(http::pagination_unknown(), limits.page_size);
        }
        let Some(has_more) = pagination.has_more else {
            // Without an explicit continuation flag the remaining scope cannot be inferred.
            state.set_pagination_diagnostic(&response, "aggregate_consistency", limits.page_size);
            state.stop(REASON_PAGINATION, false, None);
            return state.finish(http::pagination_unknown(), limits.page_size);
        };
        if !has_more {
            if state.matched_total != Some(state.kept.len() as i64) {
                state.set_pagination_diagnostic(
                    &response,
                    "aggregate_consistency",
                    limits.page_size,
                );
                state.stop(REASON_PAGINATION, false, None);
                return state.finish(http::pagination_unknown(), limits.page_size);
            }
            state.scope_complete = Some(true);
            state.truncated = false;
            state.reason = None;
            let meta = state.meta(limits.page_size);
            return Executed::ok_with_meta(state.records(), meta);
        }
        if state.pages_read as u32 >= limits.max_pages {
            state.stop(REASON_MAX_PAGES, true, Some(false));
            return state.finish(
                limit_reached("已达到本次汇总的页数上限；仍有后续数据未读取。"),
                limits.page_size,
            );
        }
        state.page = pagination.next_page.unwrap_or(state.page + 1);
    }
}
