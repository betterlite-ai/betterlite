use std::future::Future;

use secrecy::SecretString;

use crate::migration_ledger::lock::SystemJitter;
use crate::request_log_index_catalog::definitions::RequestLogIndex;

use super::database::{DatabaseFailure as _, IndexDatabase};
use super::leaf::build_leaf;
use super::locks::TokioPause;
use super::partition::build_on_partitioned_table;
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
        connect::connect(database_url),
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
        evaluate(&builder, indexes).await
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

async fn evaluate<D>(
    builder: &IndexBuilder<'_, D>,
    indexes: &[RequestLogIndex<'_>],
) -> Result<bool, D::Error>
where
    D: IndexDatabase,
{
    let mut all = true;
    for index in indexes {
        all &= ensure_index(builder, *index).await?;
    }
    Ok(all)
}

async fn ensure_index<D>(
    builder: &IndexBuilder<'_, D>,
    index: RequestLogIndex<'_>,
) -> Result<bool, D::Error>
where
    D: IndexDatabase,
{
    let Some(table) = builder
        .database
        .table_kind(builder.schema, index.table)
        .await?
    else {
        tracing::info!(
            "Table {} does not exist yet, skipping index {}",
            index.table,
            index.name
        );
        return Ok(true);
    };
    if table.partitioned {
        return build_on_partitioned_table(builder, index, None, None).await;
    }
    build_leaf(builder, index.table, index.name, index).await
}
