use std::fmt::Display;
use std::future::Future;

// A failed linked-row read costs only that table's counter invalidation; the reset itself writes by budget_id and still lands.
pub async fn fetched_or_empty<Row, Failure, Fetch, FetchFuture>(
    log_subject: &str,
    fetch: Fetch,
) -> Vec<Row>
where
    Row: Send,
    Failure: Display + Send,
    Fetch: FnOnce() -> FetchFuture + Send,
    FetchFuture: Future<Output = Result<Vec<Row>, Failure>> + Send,
{
    match fetch().await {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(
                subject = log_subject,
                error = %error,
                "Failed to fetch linked rows for counter invalidation"
            );
            Vec::new()
        }
    }
}

mod tests;
