use crate::request_log_index_catalog::catalog::regclass_name;
use crate::request_log_index_catalog::definitions::RequestLogIndex;

use super::IndexBuilder;
use super::database::{IndexDatabase, IndexRef};

pub(crate) async fn create_leaf<D>(
    builder: &IndexBuilder<'_, D>,
    target: IndexRef<'_>,
    index: RequestLogIndex<'_>,
) -> Result<bool, D::Error>
where
    D: IndexDatabase,
{
    tracing::info!(
        "Building index {} on {} concurrently",
        target.name,
        target.table
    );
    builder
        .database
        .execute(&format!(
            "CREATE INDEX CONCURRENTLY IF NOT EXISTS \"{}\" ON {} {}",
            target.name.replace('"', "\"\""),
            regclass_name(target.schema, target.table),
            index.definition
        ))
        .await?;
    let built = builder
        .database
        .index_state(target.schema, target.name)
        .await?;
    Ok(built.is_some_and(|state| state.valid))
}
