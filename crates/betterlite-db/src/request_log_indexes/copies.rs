use crate::request_log_index_catalog::catalog::EquivalentIndex;
use crate::request_log_index_catalog::definitions::RequestLogIndex;

use super::IndexBuilder;
use super::database::{IndexDatabase, IndexRef};

pub(crate) async fn equivalent_copies<D>(
    builder: &IndexBuilder<'_, D>,
    table: &str,
    name: &str,
    index: RequestLogIndex<'_>,
) -> Result<Vec<EquivalentIndex>, D::Error>
where
    D: IndexDatabase,
{
    let target = IndexRef {
        schema: builder.schema,
        table,
        name,
    };
    builder.database.equivalent_indexes(target, index).await
}
