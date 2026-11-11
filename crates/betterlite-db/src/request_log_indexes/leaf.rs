use crate::request_log_index_catalog::catalog::regclass_name;
use crate::request_log_index_catalog::definitions::RequestLogIndex;

use super::IndexBuilder;
use super::database::{DatabaseFuture, IndexDatabase};
use super::locks::{HANDOVER, under_migration_lock};

// Mirrors _build_leaf_index: CONCURRENTLY so writes keep flowing, the catalog read again under the lock
pub(crate) fn build_leaf<'leaf, D>(
    builder: &'leaf IndexBuilder<'_, D>,
    table: &'leaf str,
    name: &'leaf str,
    index: RequestLogIndex<'leaf>,
) -> DatabaseFuture<'leaf, bool, D::Error>
where
    D: IndexDatabase,
{
    Box::pin(async move {
        let current = builder.database.index_state(builder.schema, name).await?;
        let needs_lock = current
            .as_ref()
            .is_none_or(|state| !state.valid || state.table != table);
        if needs_lock && !locked_leaf_step(builder, table, name, index).await? {
            return Ok(false);
        }
        report_second_copies(builder, table, name, index, true).await?;
        Ok(true)
    })
}

async fn locked_leaf_step<D>(
    builder: &IndexBuilder<'_, D>,
    table: &str,
    name: &str,
    index: RequestLogIndex<'_>,
) -> Result<bool, D::Error>
where
    D: IndexDatabase,
{
    let locked = under_migration_lock(builder, leaf_step(builder, table, name, index)).await?;
    if locked {
        builder.dependencies.pause.pause(HANDOVER).await;
    }
    Ok(locked)
}

fn leaf_step<'leaf, D>(
    builder: &'leaf IndexBuilder<'_, D>,
    table: &'leaf str,
    name: &'leaf str,
    index: RequestLogIndex<'leaf>,
) -> DatabaseFuture<'leaf, bool, D::Error>
where
    D: IndexDatabase,
{
    Box::pin(async move {
        let Some(existing) = builder.database.index_state(builder.schema, name).await? else {
            return build_missing_leaf(builder, table, name, index).await;
        };
        if existing.table != table {
            tracing::warn!(
                "Index {name} already exists on {} rather than {table}, leaving it alone",
                existing.table
            );
            return Ok(false);
        }
        if existing.valid {
            return Ok(true);
        }
        tracing::info!("Dropping the invalid index {name} left by an interrupted build on {table}");
        let statement = format!(
            "DROP INDEX CONCURRENTLY {}",
            regclass_name(builder.schema, name)
        );
        builder.database.execute(&statement).await?;
        create_leaf(builder, table, name, index).await
    })
}

async fn build_missing_leaf<D>(
    builder: &IndexBuilder<'_, D>,
    table: &str,
    name: &str,
    index: RequestLogIndex<'_>,
) -> Result<bool, D::Error>
where
    D: IndexDatabase,
{
    if adopt_equivalent(builder, table, name, index).await? {
        return Ok(true);
    }
    create_leaf(builder, table, name, index).await
}

async fn create_leaf<D>(
    builder: &IndexBuilder<'_, D>,
    table: &str,
    name: &str,
    index: RequestLogIndex<'_>,
) -> Result<bool, D::Error>
where
    D: IndexDatabase,
{
    tracing::info!("Building index {name} on {table} concurrently");
    let statement = format!(
        "CREATE INDEX CONCURRENTLY IF NOT EXISTS \"{}\" ON {} {}",
        name.replace('"', "\"\""),
        regclass_name(builder.schema, table),
        index.definition
    );
    builder.database.execute(&statement).await?;
    let built = builder.database.index_state(builder.schema, name).await?;
    Ok(built.is_some_and(|state| state.valid))
}

pub(crate) async fn adopt_equivalent<D>(
    builder: &IndexBuilder<'_, D>,
    table: &str,
    name: &str,
    index: RequestLogIndex<'_>,
) -> Result<bool, D::Error>
where
    D: IndexDatabase,
{
    let equivalents = builder
        .database
        .equivalent_indexes(builder.schema, table, name, index)
        .await?;
    let Some(equivalent) = equivalents.into_iter().find(|candidate| candidate.valid) else {
        return Ok(false);
    };
    tracing::info!(
        "Renaming the equivalent index {} on {table} to {name} instead of building a second one",
        equivalent.name
    );
    let statement = format!(
        "ALTER INDEX {} RENAME TO \"{}\"",
        regclass_name(builder.schema, &equivalent.name),
        name.replace('"', "\"\"")
    );
    builder.database.execute(&statement).await?;
    Ok(true)
}

pub(crate) async fn report_second_copies<D>(
    builder: &IndexBuilder<'_, D>,
    table: &str,
    name: &str,
    index: RequestLogIndex<'_>,
    concurrently: bool,
) -> Result<(), D::Error>
where
    D: IndexDatabase,
{
    let copies = builder
        .database
        .equivalent_indexes(builder.schema, table, name, index)
        .await?;
    let drop = if concurrently {
        "DROP INDEX CONCURRENTLY"
    } else {
        "DROP INDEX"
    };
    for copy in copies {
        tracing::warn!(
            "Index {} on {table} is a second copy of {name} and only costs writes and disk; remove it with: {drop} {}",
            copy.name,
            regclass_name(builder.schema, &copy.name)
        );
    }
    Ok(())
}
