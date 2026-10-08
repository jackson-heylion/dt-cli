use super::*;
use tokio::time::{Instant, timeout_at};

fn stopped(mut failure: Failure, id: &str, last: &Value) -> Failure {
    if failure.trace_id.is_none() {
        failure.trace_id = last["_meta"]["traceId"].as_str().map(str::to_owned);
    }
    let mut meta = failure
        .partial_meta
        .as_deref()
        .cloned()
        .unwrap_or_else(|| json!({}));
    meta["complete"] = json!(false);
    failure.recovery = Some(Box::new(
        json!({"jobId":id,"state":last["state"],"complete":false}),
    ));
    failure.with_partial(
        json!({"jobId":id,"state":last["state"],"complete":false}),
        meta,
    )
}

/// Observes one existing read job under a single budget and never submits or cancels it.
pub(super) async fn wait_job(rt: &Runtime, name: &str, id: &str, seconds: u64) -> Result<Value> {
    if !job_id(id) || !(1..=600).contains(&seconds) {
        return Err(invalid());
    }
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let p = require_profile(rt, name)?;
    let env = rt.environment(&p.environment)?;
    let client = http::client()?;
    wait_with(rt, &p, env, &client, id, deadline).await
}

pub(super) async fn wait_with(
    rt: &Runtime,
    p: &GovernedProfile,
    env: &profile::Environment,
    client: &reqwest::Client,
    id: &str,
    deadline: Instant,
) -> Result<Value> {
    let mut last = json!({"jobId":id,"state":"unknown"});
    let mut delay = Duration::from_secs(2);
    let cancelled = tokio::signal::ctrl_c();
    tokio::pin!(cancelled);
    loop {
        let query = job_action_with(rt, p, env, client, "jobs.status", id);
        let observed = tokio::select! {biased;
            _=&mut cancelled=>return Err(stopped(Failure::new("WAIT_CANCELLED",8,"已停止观察；服务端任务没有被取消。"),id,&last)),
            result=timeout_at(deadline,query)=>match result {
                Ok(value)=>value,
                Err(_)=>return Err(stopped(Failure::new("JOB_WAIT_TIMEOUT",7,"等待预算已用完；请继续查询原任务。"),id,&last)),
            },
        };
        let wait = match observed {
            Ok(value) => {
                if !matches!(
                    value["state"].as_str(),
                    Some("queued" | "running" | "succeeded" | "failed" | "cancelled")
                ) {
                    return Err(stopped(http::protocol(), id, &last));
                }
                if rt.interactive && value["state"] != last["state"] {
                    eprintln!(
                        "读取任务状态：{}",
                        value["state"].as_str().unwrap_or("unknown")
                    );
                }
                last = value;
                match last["state"].as_str() {
                    Some("succeeded") => {
                        let result = tokio::select! {biased;
                            _=&mut cancelled=>Err(Failure::new("WAIT_CANCELLED",8,"已停止观察；服务端任务没有被取消。")),
                            result=timeout_at(deadline,job_action_with(rt,p,env,client,"jobs.result",id))=>result.unwrap_or_else(|_|Err(Failure::new("JOB_WAIT_TIMEOUT",7,"结果下载超过总预算；原任务仍可查询。"))),
                        };
                        return result.map_err(|failure| stopped(failure, id, &last));
                    }
                    Some("queued" | "running") => last["_meta"]["retryAfterSeconds"]
                        .as_u64()
                        .map(Duration::from_secs)
                        .unwrap_or(delay)
                        .max(delay),
                    Some("failed" | "cancelled") => {
                        return Err(stopped(partial_result(), id, &last));
                    }
                    _ => return Err(stopped(http::protocol(), id, &last)),
                }
            }
            Err(failure) => {
                let retry = failure
                    .partial_meta
                    .as_deref()
                    .and_then(|m| m["retryAfterSeconds"].as_u64());
                if matches!(failure.code, "RATE_LIMITED" | "DEPENDENCY_UNAVAILABLE")
                    && let Some(seconds) = retry
                {
                    Duration::from_secs(seconds).max(delay)
                } else {
                    return Err(stopped(failure, id, &last));
                }
            }
        };
        tokio::select! {biased;
            _=&mut cancelled=>return Err(stopped(Failure::new("WAIT_CANCELLED",8,"已停止观察；服务端任务没有被取消。"),id,&last)),
            result=timeout_at(deadline,tokio::time::sleep(wait))=>if result.is_err(){return Err(stopped(Failure::new("JOB_WAIT_TIMEOUT",7,"等待预算已用完；请继续查询原任务。"),id,&last));},
        }
        delay = (delay * 2).min(Duration::from_secs(10));
    }
}
