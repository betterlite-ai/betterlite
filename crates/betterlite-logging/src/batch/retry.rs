pub const RETRYABLE_CLIENT_STATUS_CODES: [u16; 2] = [408, 429];

// LiteLLM requeues every status outside 4xx, plus the client errors a retry can clear
#[must_use]
pub fn is_retryable_status(status: u16) -> bool {
    match status {
        400..=499 => RETRYABLE_CLIENT_STATUS_CODES.contains(&status),
        _ => true,
    }
}

#[must_use]
pub fn undelivered_after_http_error<E>(batch: &[E], status: u16) -> Vec<E>
where
    E: Clone,
{
    if is_retryable_status(status) {
        batch.to_vec()
    } else {
        Vec::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequeuePolicy {
    RequeueAll,
    Undelivered,
}

impl RequeuePolicy {
    #[must_use]
    pub fn retained<E>(self, batch: &[E], status: u16) -> Vec<E>
    where
        E: Clone,
    {
        match self {
            Self::RequeueAll => batch.to_vec(),
            Self::Undelivered => undelivered_after_http_error(batch, status),
        }
    }
}

mod tests;
