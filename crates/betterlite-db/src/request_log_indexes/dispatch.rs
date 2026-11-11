use crate::request_log_index_catalog::definitions::RequestLogIndex;

use super::IndexBuilder;
use super::database::{IndexDatabase, IndexRef};
use super::leaf::build_leaf;
use super::partition::{PartitionTarget, build_on_partitioned_table};

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
        let partition = PartitionTarget {
            schema: builder.schema,
            table: None,
            name: None,
        };
        return build_on_partitioned_table(builder, index, partition).await;
    }
    let target = IndexRef {
        schema: builder.schema,
        table: index.table,
        name: index.name,
    };
    build_leaf(builder, target, index).await
}
