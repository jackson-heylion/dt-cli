use super::*;

pub struct Cancel {
    source: Pin<Box<dyn Future<Output = ()> + Send>>,
}
impl Cancel {
    pub fn signal() -> Self {
        Self {
            source: Box::pin(async {
                let _ = tokio::signal::ctrl_c().await;
            }),
        }
    }
    pub fn disabled() -> Self {
        Self {
            source: Box::pin(std::future::pending()),
        }
    }
    pub fn channel(receiver: tokio::sync::oneshot::Receiver<()>) -> Self {
        Self {
            source: Box::pin(async move {
                let _ = receiver.await;
            }),
        }
    }
    /// Runs `future` under cancellation and the absolute operation deadline.
    pub async fn race<T>(
        &mut self,
        deadline: Instant,
        future: impl Future<Output = T>,
    ) -> std::result::Result<T, Failure> {
        tokio::select! {
            biased;
            _ = &mut self.source => Err(cancelled()),
            r = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), future) => {
                r.map_err(|_| deadline_reached())
            }
        }
    }
    pub async fn wait(
        &mut self,
        deadline: Instant,
        duration: Duration,
    ) -> std::result::Result<(), Failure> {
        self.race(deadline, tokio::time::sleep(duration)).await
    }
}

pub enum Attempt {
    Page(Box<PageResponse>),
    Stopped(Failure),
}

/// One page request. The single resend budget is shared by transient transport failures,
/// 429/503 with a usable Retry-After, and one credential recovery after explicit expiry.
#[allow(clippy::too_many_arguments)]
pub async fn fetch_page(
    rt: &Runtime,
    cancel: &mut Cancel,
    deadline: Instant,
    client: &reqwest::Client,
    profile: &Profile,
    env: &Environment,
    credentials: &mut Credentials,
    params: &Value,
    allowance: usize,
) -> Attempt {
    // Exactly one branch can resend, so the single resend budget is shared by construction
    // between transport failures, a usable Retry-After and one credential recovery.
    let mut response = match cancel
        .race(
            deadline,
            http::workflow_page(
                client,
                env,
                credentials,
                params,
                allowance.min(http::PAGE_BYTE_LIMIT),
            ),
        )
        .await
    {
        Ok(response) => response,
        Err(stop) => return Attempt::Stopped(stop),
    };
    if response
        .failure
        .as_ref()
        .is_some_and(|failure| failure.code == "RESULT_LIMIT")
    {
        return Attempt::Page(Box::new(response));
    }
    let retry = if response.expired {
        match cancel
            .race(
                deadline,
                rt.recover_credentials_after(profile, env, client, credentials),
            )
            .await
        {
            Ok(Ok(refreshed)) => {
                *credentials = refreshed;
                true
            }
            Ok(Err(failure)) => {
                response.failure = Some(failure);
                return Attempt::Page(Box::new(response));
            }
            Err(stop) => return Attempt::Stopped(stop),
        }
    } else if let Some(wait) = response.retry_after {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if let Some(seconds) = wait
            && Duration::from_secs(seconds) <= remaining
        {
            if let Err(stop) = cancel.wait(deadline, Duration::from_secs(seconds)).await {
                return Attempt::Stopped(stop);
            }
            true
        } else {
            false
        }
    } else {
        response.status == 0
    };
    if retry {
        let remaining = allowance.saturating_sub(response.bytes);
        if remaining == 0 {
            response.failure = Some(http::over_limit());
            response.content_truncated = true;
        } else {
            match cancel
                .race(
                    deadline,
                    http::workflow_page(
                        client,
                        env,
                        credentials,
                        params,
                        remaining.min(http::PAGE_BYTE_LIMIT),
                    ),
                )
                .await
            {
                Ok(mut retried) => {
                    // Failed responses count too: retrying must not reset the byte budget.
                    retried.bytes += response.bytes;
                    response = retried;
                }
                Err(stop) => return Attempt::Stopped(stop),
            }
        }
    }
    Attempt::Page(Box::new(response))
}
