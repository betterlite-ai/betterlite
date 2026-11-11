use crate::request_log_index_catalog::definitions::RequestLogIndex;

use super::IndexBuilder;
use super::database::IndexDatabase;
use super::leaf::build_leaf;
use super::partition::build_on_partitioned_table;

pub(crate) async fn ensure_index<D>(
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
