use super::{DbError, ErrorType, is_database_transport_error, is_error_type_or_subclass};
use serde::{Deserialize, de::IgnoredAny};
use std::future::Future;
use std::pin::Pin;

#[derive(Deserialize)]
#[serde(untagged)]
pub enum Timeout {
    Number(f64),
    Invalid(IgnoredAny),
}
impl Timeout {
    #[must_use]
    pub const fn coerce(&self, fallback: f64) -> f64 {
        match self {
            Self::Number(value) => *value,
            Self::Invalid(_) => fallback,
        }
    }
}

pub trait DbReconnect {
    fn reconnect_timeout_seconds(&self) -> Option<f64>;

    fn reconnect_lock_timeout_seconds(&self) -> Option<f64>;

    fn attempt_db_reconnect<'client>(
        &'client mut self,
        reason: &'client str,
        timeout_seconds: f64,
        lock_timeout_seconds: f64,
    ) -> Pin<Box<dyn Future<Output = Result<bool, DbError>> + Send + 'client>>;
}

pub struct RetryPolicy<'reason> {
    pub reason: &'reason str,
    pub retry_safe_error_types: Option<&'reason [ErrorType]>,
    pub timeout_seconds: Option<f64>,
    pub lock_timeout_seconds: Option<f64>,
}

fn retry_timeouts<Client>(client: &Client, policy: &RetryPolicy<'_>) -> (f64, f64)
where
    Client: DbReconnect,
{
    let timeout_seconds = policy
        .timeout_seconds
        .or_else(|| client.reconnect_timeout_seconds())
        .unwrap_or(2.0_f64);
    let lock_timeout_seconds = policy
        .lock_timeout_seconds
        .or_else(|| client.reconnect_lock_timeout_seconds())
        .unwrap_or(0.1_f64);
    (timeout_seconds, lock_timeout_seconds)
}

fn is_retry_eligible(error: &DbError, policy: &RetryPolicy<'_>) -> bool {
    if !is_database_transport_error(error) {
        return false;
    }
    let Some(allowed) = policy.retry_safe_error_types else {
        return true;
    };
    allowed
        .iter()
        .any(|&ancestor| is_error_type_or_subclass(error.error_type, ancestor))
}

pub async fn call_with_db_reconnect_retry<T, Read, ReadFuture, Client>(
    client: Option<&mut Client>,
    mut read: Read,
    policy: RetryPolicy<'_>,
) -> Result<T, DbError>
where
    Read: FnMut() -> ReadFuture + Send,
    ReadFuture: Future<Output = Result<T, DbError>> + Send,
    Client: DbReconnect + Send,
    T: Send,
{
    let error = match read().await {
        Ok(value) => return Ok(value),
        Err(error) => error,
    };
    if !is_retry_eligible(&error, &policy) {
        return Err(error);
    }
    let Some(available_client) = client else {
        return Err(error);
    };
    let (timeout_seconds, lock_timeout_seconds) = retry_timeouts(available_client, &policy);
    tracing::warn!(
        reason = policy.reason,
        error = %error,
        "DB transport error on read; attempting reconnect-and-retry"
    );
    match available_client
        .attempt_db_reconnect(policy.reason, timeout_seconds, lock_timeout_seconds)
        .await
    {
        Ok(true) => read().await,
        Ok(false) => Err(error),
        Err(reconnect_error) => {
            tracing::warn!(
                reason = policy.reason,
                reconnect_error = %reconnect_error,
                "DB reconnect attempt raised; preserving original transport error"
            );
            Err(DbError {
                cause: Some(Box::new(reconnect_error)),
                ..error
            })
        }
    }
}
