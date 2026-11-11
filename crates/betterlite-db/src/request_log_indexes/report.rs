use crate::request_log_index_catalog::catalog::regclass_name;
use crate::request_log_index_catalog::definitions::RequestLogIndex;

use super::IndexBuilder;
use super::copies::equivalent_copies;
use super::database::IndexDatabase;

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
    let copies = equivalent_copies(builder, table, name, index).await?;
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
