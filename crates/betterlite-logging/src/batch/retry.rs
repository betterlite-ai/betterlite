pub const RETRYABLE_CLIENT_STATUS_CODES: [u16; 2] = [408, 429];

// LiteLLM requeues every status outside 4xx, plus the client errors a retry can clear
#[must_use]
pub fn is_retryable_status(status: u16) -> bool {
    match status {
        400..=499 => RETRYABLE_CLIENT_STATUS_CODES.contains(&status),
        _ => true,
    }
}

fn log_status(records: usize, status: u16, integration_name: &str, detail: &str, retried: bool) {
    if retried {
        tracing::error!(
            integration = integration_name,
            status,
            records,
            detail,
            "the destination answered with a status a retry can clear; the records will be retried"
        );
        return;
    }
    tracing::error!(
        integration = integration_name,
        status,
        records,
        detail,
        "the destination rejected the records with a status a retry cannot clear; the records are dropped"
    );
}

#[must_use]
pub fn undelivered_after_http_error<E>(
    batch: &[E],
    status: u16,
    integration_name: &str,
    detail: &str,
) -> Vec<E>
where
    E: Clone,
{
    let retried = is_retryable_status(status);
    log_status(batch.len(), status, integration_name, detail, retried);
    if retried { batch.to_vec() } else { Vec::new() }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequeuePolicy {
    RequeueAll,
    Undelivered,
}

impl RequeuePolicy {
    #[must_use]
    pub fn retained<E>(
        self,
        batch: &[E],
        status: u16,
        integration_name: &str,
        detail: &str,
    ) -> Vec<E>
    where
        E: Clone,
    {
        match self {
            Self::RequeueAll => {
                log_status(batch.len(), status, integration_name, detail, true);
                batch.to_vec()
            }
            Self::Undelivered => {
                undelivered_after_http_error(batch, status, integration_name, detail)
            }
        }
    }
}

mod tests;
