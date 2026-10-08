//! Bounded multi-page aggregation for `workflow list --all`.
//!
//! The loop deliberately stays out of the command dispatcher: the dedicated command and
//! `api call iam.workflow.list` both enter here, so an alias call cannot drift from it.
use crate::{
    Runtime,
    credentials::Credentials,
    http::{self, PageResponse, Pagination},
    output::{Executed, Failure},
    profile::{Environment, Profile},
};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    future::Future,
    pin::Pin,
    time::{Duration, Instant},
};

/// The whole aggregation, including lock waits, refreshes, reads, retries and waits.
pub const AGGREGATE_SECONDS: u64 = 30;
/// Largest number of pages one aggregation may read.
pub const MAX_PAGES_LIMIT: u32 = 20;
/// Largest number of unique records one aggregation may return.
pub const MAX_ITEMS_LIMIT: usize = 1000;

pub const REASON_MAX_PAGES: &str = "max-pages";
pub const REASON_MAX_ITEMS: &str = "max-items";
pub const REASON_BYTES: &str = "bytes";
pub const REASON_DEADLINE: &str = "deadline";
pub const REASON_DATA_CHANGED: &str = "data-changed";
pub const REASON_PAGINATION: &str = "pagination-unknown";
pub const REASON_CANCELLED: &str = "cancelled";
pub const REASON_PARTIAL: &str = "partial-result";

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub page_size: i64,
    pub max_pages: u32,
    pub max_items: usize,
}

fn cancelled() -> Failure {
    Failure::new("LOGIN_CANCELLED", 8, "操作已取消。")
}
fn deadline_reached() -> Failure {
    Failure::new("TIMEOUT", 5, "读取超过本次操作的总时间预算。")
}
fn partial() -> Failure {
    Failure::new(
        "PARTIAL_RESULT",
        7,
        "后续页未能读取；已保留的页面完整可用。",
    )
}
fn data_changed() -> Failure {
    Failure::new(
        "DATA_CHANGED",
        7,
        "读取期间分页数据发生变化；已保留首次出现的记录。",
    )
}
fn limit_reached(message: &'static str) -> Failure {
    Failure::new("RESULT_LIMIT", 7, message)
}
/// A dead credential is terminal even mid-aggregation: no further page can succeed.
fn auth_class(code: &str) -> bool {
    matches!(
        code,
        "AUTH_REQUIRED"
            | "ACCESS_TOKEN_EXPIRED"
            | "AUTHORIZATION_EXPIRED"
            | "AUTHORIZATION_REVOKED"
            | "AUTH_DENIED"
            | "IDENTITY_MISMATCH"
    )
}

/// One listener, raced against every await point of a single operation.
mod fetch;
pub use fetch::{Attempt, Cancel, fetch_page};

mod aggregation;
pub use aggregation::aggregate;
pub(crate) use aggregation::{Bounds, aggregate_bounded};

pub struct Problem {
    pub pagination: bool,
    pub content: bool,
}

/// Per-page contract checks. Anything that cannot be proven complete is refused here.
/// `follow_up` marks a page requested because the previous page reported more data.
pub fn page_problem(
    pagination: &Pagination,
    requested: i64,
    page_size: i64,
    response: &PageResponse,
    follow_up: bool,
) -> Option<Problem> {
    let count = response.data.len() as i64;
    let pagination_problem = pagination.mode != "page"
        || pagination.page != requested
        || pagination.page_size != page_size
        || pagination.pages_read != 1
        || !pagination.page_complete
        || pagination.items_fetched != count
        || pagination.items_returned != count
        || pagination.truncated
        || pagination.reason.is_some()
        || pagination.matched_total.is_some_and(|total| total < 0)
        || pagination.scope_complete == Some(true)
            && (requested != 1 || pagination.has_more != Some(false))
        || count > page_size
        || pagination.has_more == Some(true) && pagination.next_page != Some(requested + 1)
        || pagination.has_more == Some(false) && pagination.next_page.is_some()
        || count == 0 && (follow_up || pagination.has_more == Some(true));
    if pagination_problem {
        return Some(Problem {
            pagination: true,
            content: false,
        });
    }
    if !pagination.content_complete {
        return Some(Problem {
            pagination: false,
            content: true,
        });
    }
    None
}
