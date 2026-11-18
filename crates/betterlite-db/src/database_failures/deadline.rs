use std::future::Future;
use std::time::Duration;

use tokio::task::{AbortHandle, JoinError, JoinHandle};
use tokio::time::timeout;
use tokio_util::task::TaskTracker;

use crate::database_faults::DbError;

use super::gate::{DbLookupStallTracker, LookupClock, SystemClock};

#[derive(Debug, thiserror::Error)]
#[error(
    "{} lookup did not answer within {}s",
    .lookup,
    deadline_text(.deadline_seconds)
)]
pub struct DbLookupDeadlineExceeded {
    pub lookup: String,
    pub deadline_seconds: f64,
    #[source]
    pub cause: Option<Box<DbLookupError>>,
}

#[derive(Debug, thiserror::Error)]
pub enum DbLookupError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Deadline(#[from] DbLookupDeadlineExceeded),
    #[error(transparent)]
    Elapsed(#[from] tokio::time::error::Elapsed),
    #[error(transparent)]
    Task(JoinError),
}

pub struct LookupRequest<'request, Clock = SystemClock> {
    pub name: &'request str,
    pub deadline_seconds: Option<f64>,
    pub default_deadline_seconds: f64,
    pub tracker: &'request DbLookupStallTracker<Clock>,
}

pub async fn bounded_db_lookup<Lookup, Output, Clock>(
    lookup: Lookup,
    request: LookupRequest<'_, Clock>,
    tasks: &TaskTracker,
) -> Result<Output, DbLookupError>
where
    Lookup: Future<Output = Result<Output, DbLookupError>> + Send + 'static,
    Output: Send + 'static,
    Clock: LookupClock,
{
    let deadline_seconds = request
        .deadline_seconds
        .unwrap_or(request.default_deadline_seconds);
    let handle = tasks.spawn(lookup);
    let _cancel = CancelOnDrop(handle.abort_handle());
    let waited = match lookup_budget(deadline_seconds) {
        LookupBudget::Forever => None,
        LookupBudget::Within(budget) => Some(timeout(budget, &mut handle).await),
    };
    let joined = match waited {
        Some(Ok(joined)) => joined,
        Some(Err(_elapsed)) => {
            handle.abort();
            drop(tasks.spawn(consume_abandoned_lookup(handle)));
            request.tracker.record_hit();
            return Err(deadline_error(request.name, deadline_seconds, None));
        }
        None => handle.await,
    };
    completed_lookup(joined, request.name, deadline_seconds, request.tracker)
}

fn completed_lookup<Output, Clock>(
    joined: Result<Result<Output, DbLookupError>, JoinError>,
    name: &str,
    deadline_seconds: f64,
    tracker: &DbLookupStallTracker<Clock>,
) -> Result<Output, DbLookupError>
where
    Clock: LookupClock,
{
    match joined {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(DbLookupError::Deadline(existing))) => Err(DbLookupError::Deadline(existing)),
        Ok(Err(DbLookupError::Elapsed(source))) => {
            tracker.record_hit();
            Err(deadline_error(
                name,
                deadline_seconds,
                Some(Box::new(DbLookupError::Elapsed(source))),
            ))
        }
        Ok(Err(other)) => Err(other),
        Err(join_error) => Err(DbLookupError::Task(join_error)),
    }
}

pub(super) async fn consume_abandoned_lookup<Output>(
    handle: JoinHandle<Result<Output, DbLookupError>>,
) where
    Output: Send,
{
    if let Err(join_error) = handle.await
        && !join_error.is_cancelled()
    {
        tracing::debug!(%join_error, "an abandoned database lookup failed");
    }
}

fn deadline_error(
    name: &str,
    deadline_seconds: f64,
    cause: Option<Box<DbLookupError>>,
) -> DbLookupError {
    DbLookupError::Deadline(DbLookupDeadlineExceeded {
        lookup: name.to_owned(),
        deadline_seconds,
        cause,
    })
}

struct CancelOnDrop(AbortHandle);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

enum LookupBudget {
    Forever,
    Within(Duration),
}

fn lookup_budget(deadline_seconds: f64) -> LookupBudget {
    if deadline_seconds <= 0.0 {
        return LookupBudget::Within(Duration::ZERO);
    }
    match Duration::try_from_secs_f64(deadline_seconds) {
        Ok(budget) => LookupBudget::Within(budget),
        Err(_too_large) => LookupBudget::Forever,
    }
}

pub(super) fn deadline_text(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_owned();
    }
    if value.is_infinite() {
        return if value.is_sign_negative() {
            "-inf".to_owned()
        } else {
            "inf".to_owned()
        };
    }
    let scientific = format!("{:.5e}", value.abs());
    let (mantissa, exponent_text) = scientific.split_once('e').unwrap_or_default();
    let exponent = exponent_text.parse::<i32>().unwrap_or_default();
    let digits: String = mantissa
        .chars()
        .filter(|character| *character != '.')
        .collect();
    let significant = digits.trim_end_matches('0');
    let body = if significant.is_empty() {
        "0".to_owned()
    } else if (-4..6).contains(&exponent) {
        fixed_notation(significant, exponent)
    } else {
        scientific_notation(significant, exponent)
    };
    if value.is_sign_negative() {
        format!("-{body}")
    } else {
        body
    }
}

fn fixed_notation(digits: &str, exponent: i32) -> String {
    if exponent >= 0 {
        let point = usize::try_from(exponent.saturating_add(1)).unwrap_or(usize::MAX);
        if let Some((integer, fraction)) = digits.split_at_checked(point) {
            return if fraction.is_empty() {
                integer.to_owned()
            } else {
                format!("{integer}.{fraction}")
            };
        }
        let mut body = digits.to_owned();
        body.push_str(&"0".repeat(point.saturating_sub(digits.len())));
        return body;
    }
    let zeros = usize::try_from(exponent.saturating_neg().saturating_sub(1)).unwrap_or_default();
    let mut body = String::from("0.");
    body.push_str(&"0".repeat(zeros));
    body.push_str(digits);
    body
}

fn scientific_notation(digits: &str, exponent: i32) -> String {
    let Some((first, rest)) = digits.split_at_checked(1) else {
        return "0".to_owned();
    };
    let mantissa = if rest.is_empty() {
        first.to_owned()
    } else {
        format!("{first}.{rest}")
    };
    let sign = if exponent.is_negative() { '-' } else { '+' };
    let magnitude = exponent.unsigned_abs();
    format!("{mantissa}e{sign}{magnitude:02}")
}
