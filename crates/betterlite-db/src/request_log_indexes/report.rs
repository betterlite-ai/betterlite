use crate::request_log_index_catalog::catalog::regclass_name;
use crate::request_log_index_catalog::definitions::RequestLogIndex;

use super::IndexBuilder;
use super::database::{IndexDatabase, IndexRef};

pub(crate) async fn report_second_copies<D>(
    builder: &IndexBuilder<'_, D>,
    target: IndexRef<'_>,
    index: RequestLogIndex<'_>,
    concurrently: bool,
) -> Result<(), D::Error>
where
    D: IndexDatabase,
{
    let copies = builder.database.equivalent_indexes(target, index).await?;
    let drop = if concurrently {
        "DROP INDEX CONCURRENTLY"
    } else {
        "DROP INDEX"
    };
    for copy in copies {
        tracing::warn!(
            "Index {} on {} is a second copy of {} and only costs writes and disk; remove it with: {drop} {}",
            copy.name,
            target.table,
            target.name,
            regclass_name(target.schema, &copy.name)
        );
    }
    Ok(())
}
