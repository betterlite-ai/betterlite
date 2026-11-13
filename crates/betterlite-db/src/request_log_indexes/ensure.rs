use std::future::Future;

use secrecy::SecretString;

use crate::migration_ledger::lock::SystemJitter;
use crate::request_log_index_catalog::definitions::RequestLogIndex;

use super::database::{DatabaseFailure as _, IndexDatabase};
use super::dispatch::ensure_index;
use super::locks::TokioPause;
use super::{IndexBuilder, IndexDependencies, connect};

pub async fn ensure_request_log_indexes(
    database_url: &SecretString,
    schema: &str,
    indexes: &[RequestLogIndex<'_>],
) -> bool {
    let jitter = match SystemJitter::try_new() {
        Ok(jitter) => jitter,
        Err(error) => {
            tracing::error!(
                "Could not build the request-log indexes, leaving them for the next index build: {error}"
            );
            return false;
        }
    };
    let pause = TokioPause;
    let dependencies = IndexDependencies {
        jitter: &jitter,
        pause: &pause,
    };
    match ensure_indexes(
        connect::connect(database_url, connect::CONNECT_TIMEOUT),
        schema,
        indexes,
        &dependencies,
    )
    .await
    {
        Ok(result) => result,
        Err(error) => {
            tracing::error!(
                "Could not build the request-log indexes, leaving them for the next index build: {error}"
            );
            false
        }
    }
}

pub(crate) async fn ensure_indexes<D, F>(
    connect: F,
    schema: &str,
    indexes: &[RequestLogIndex<'_>],
    dependencies: &IndexDependencies<'_>,
) -> Result<bool, D::Error>
where
    D: IndexDatabase,
    F: Future<Output = Result<D, D::Error>>,
{
    let outcome = async {
        let database = connect.await?;
        let builder = IndexBuilder {
            database: &database,
            schema,
            dependencies,
        };
        let mut outcomes = Vec::with_capacity(indexes.len());
        for index in indexes {
            outcomes.push(ensure_index(&builder, *index).await?);
        }
        Ok::<bool, D::Error>(outcomes.into_iter().all(|kept| kept))
    }
    .await;
    match outcome {
        Ok(true) => {
            tracing::info!("Request-log indexes are all in place");
            Ok(true)
        }
        Ok(false) => {
            tracing::warn!(
                "Some request-log indexes are not in place yet, leaving them for the next index build"
            );
            Ok(false)
        }
        Err(error) if error.is_database_failure() => {
            tracing::warn!(
                "Could not build the request-log indexes, leaving them for the next index build: {error}"
            );
            Ok(false)
        }
        Err(error) => Err(error),
    }
}
