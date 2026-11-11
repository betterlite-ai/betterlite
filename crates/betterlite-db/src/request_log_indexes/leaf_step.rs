use crate::request_log_index_catalog::catalog::regclass_name;
use crate::request_log_index_catalog::definitions::RequestLogIndex;

use super::IndexBuilder;
use super::adopt::adopt_equivalent;
use super::create::create_leaf;
use super::database::IndexDatabase;

pub(crate) async fn leaf_step<D>(
    builder: &IndexBuilder<'_, D>,
    table: &str,
    name: &str,
    index: RequestLogIndex<'_>,
) -> Result<bool, D::Error>
where
    D: IndexDatabase,
{
    let Some(existing) = builder.database.index_state(builder.schema, name).await? else {
        if adopt_equivalent(builder, table, name, index).await? {
            return Ok(true);
        }
        return create_leaf(builder, table, name, index).await;
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
}
