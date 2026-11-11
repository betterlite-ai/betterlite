use crate::request_log_index_catalog::catalog::regclass_name;
use crate::request_log_index_catalog::definitions::RequestLogIndex;

use super::IndexBuilder;
use super::adopt::adopt_equivalent;
use super::create::create_leaf;
use super::database::{IndexDatabase, IndexRef};

pub(crate) async fn leaf_step<D>(
    builder: &IndexBuilder<'_, D>,
    target: IndexRef<'_>,
    index: RequestLogIndex<'_>,
) -> Result<bool, D::Error>
where
    D: IndexDatabase,
{
    let Some(existing) = builder
        .database
        .index_state(target.schema, target.name)
        .await?
    else {
        if adopt_equivalent(builder, target, index).await? {
            return Ok(true);
        }
        return create_leaf(builder, target, index).await;
    };
    if existing.table != target.table {
        tracing::warn!(
            "Index {} already exists on {} rather than {}, leaving it alone",
            target.name,
            existing.table,
            target.table
        );
        return Ok(false);
    }
    if existing.valid {
        return Ok(true);
    }
    tracing::info!(
        "Dropping the invalid index {} left by an interrupted build on {}",
        target.name,
        target.table
    );
    let statement = format!(
        "DROP INDEX CONCURRENTLY {}",
        regclass_name(target.schema, target.name)
    );
    builder.database.execute(&statement).await?;
    create_leaf(builder, target, index).await
}
