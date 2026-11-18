use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use deadpool_postgres::Client;
use tokio::time::timeout;
use tokio_postgres::tls::{MakeTlsConnect, TlsConnect};
use tokio_postgres::{CancelToken, Socket};

#[derive(Clone, Copy)]
pub struct StatementBounds<'bounds> {
    pub deadline: Duration,
    pub canceller: &'bounds dyn StatementCanceller,
}

pub type CancellationFuture<'token> =
    Pin<Box<dyn Future<Output = Result<(), tokio_postgres::Error>> + Send + 'token>>;

// A CancelToken carries the connection's SSL mode, so only a matching connector can deliver a cancel.
pub trait StatementCanceller: Send + Sync {
    fn cancel<'token>(&self, token: &'token CancelToken) -> CancellationFuture<'token>;
}

pub struct TlsCanceller<Tls> {
    pub tls: Tls,
}

impl<Tls> StatementCanceller for TlsCanceller<Tls>
where
    Tls: MakeTlsConnect<Socket> + Clone + Sync + Send + 'static,
    Tls::Stream: Send,
    Tls::TlsConnect: Send,
    <Tls::TlsConnect as TlsConnect<Socket>>::Future: Send,
{
    fn cancel<'token>(&self, token: &'token CancelToken) -> CancellationFuture<'token> {
        let tls = self.tls.clone();
        Box::pin(async move { token.cancel_query(tls).await })
    }
}

#[derive(Debug, thiserror::Error)]
pub(super) enum StatementFailure {
    #[error("{}", driver_detail(.0))]
    Driver(#[source] tokio_postgres::Error),
    #[error("the statement did not finish within {deadline:?}")]
    Expired { deadline: Duration, cancelled: bool },
}

// The driver prints only "db error"; the server's severity and message name the failing constraint, while detail and hint can carry row values.
fn driver_detail(source: &tokio_postgres::Error) -> String {
    source.as_db_error().map_or_else(
        || source.to_string(),
        |server| format!("{}: {}", server.severity(), server.message()),
    )
}

// A statement past its deadline keeps running on the server, so its client leaves the pool unless the cancel landed.
pub(super) fn discard_uncancelled(client: Client, failure: &StatementFailure) {
    if let StatementFailure::Expired {
        cancelled: false, ..
    } = failure
    {
        let detached = deadpool_postgres::Object::take(client);
        drop(detached);
    }
}

// Dropping a query future leaves the server executing the statement, so the deadline sends a cancel.
pub(super) async fn bounded_statement<Work, T>(
    client: &tokio_postgres::Client,
    bounds: StatementBounds<'_>,
    work: Work,
) -> Result<T, StatementFailure>
where
    Work: Future<Output = Result<T, tokio_postgres::Error>> + Send,
    T: Send,
{
    match timeout(bounds.deadline, work).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(StatementFailure::Driver(error)),
        Err(_deadline_elapsed) => {
            let cancelled = cancel_statement(client, bounds).await;
            Err(StatementFailure::Expired {
                deadline: bounds.deadline,
                cancelled,
            })
        }
    }
}

pub(super) async fn cancel_statement(
    client: &tokio_postgres::Client,
    bounds: StatementBounds<'_>,
) -> bool {
    let token = client.cancel_token();
    match timeout(bounds.deadline, bounds.canceller.cancel(&token)).await {
        Ok(Ok(())) => true,
        Ok(Err(error)) => {
            tracing::warn!(
                "tool_registry_writer: the timed out statement could not be cancelled: {error}"
            );
            false
        }
        Err(_cancel_elapsed) => {
            tracing::warn!(
                "tool_registry_writer: cancelling the timed out statement did not answer in time"
            );
            false
        }
    }
}
