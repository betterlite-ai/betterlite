use std::future::Future;
use std::time::Duration;

use tokio::time::timeout;
use tokio_postgres::NoTls;

#[derive(Debug, thiserror::Error)]
pub(super) enum StatementFailure {
    #[error("{0}")]
    Driver(#[source] tokio_postgres::Error),
    #[error("the statement did not finish within {0:?}")]
    Expired(Duration),
}

// Runs one statement against the deadline and asks the server to cancel it when the deadline passes.
pub(super) async fn bounded_statement<Work, T>(
    client: &tokio_postgres::Client,
    deadline: Duration,
    work: Work,
) -> Result<T, StatementFailure>
where
    Work: Future<Output = Result<T, tokio_postgres::Error>> + Send,
    T: Send,
{
    match timeout(deadline, work).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(StatementFailure::Driver(error)),
        Err(_elapsed) => {
            cancel_statement(client, deadline).await;
            Err(StatementFailure::Expired(deadline))
        }
    }
}

pub(super) async fn cancel_statement(client: &tokio_postgres::Client, deadline: Duration) {
    let token = client.cancel_token();
    match timeout(deadline, token.cancel_query(NoTls)).await {
        Ok(Ok(())) => (),
        Ok(Err(error)) => {
            tracing::warn!(
                "tool_registry_writer: the timed out statement could not be cancelled: {error}"
            );
        }
        Err(_cancel_elapsed) => {
            tracing::warn!(
                "tool_registry_writer: cancelling the timed out statement did not answer in time"
            );
        }
    }
}
