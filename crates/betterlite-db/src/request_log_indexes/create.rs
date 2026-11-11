use crate::request_log_index_catalog::catalog::regclass_name;
use crate::request_log_index_catalog::definitions::RequestLogIndex;

use super::IndexBuilder;
use super::database::IndexDatabase;

pub(crate) async fn create_leaf<D>(
    builder: &IndexBuilder<'_, D>,
    table: &str,
    name: &str,
    index: RequestLogIndex<'_>,
) -> Result<bool, D::Error>
where
    D: IndexDatabase,
{
    tracing::info!("Building index {name} on {table} concurrently");
    builder
        .database
        .execute(&format!(
            "CREATE INDEX CONCURRENTLY IF NOT EXISTS \"{}\" ON {} {}",
            name.replace('"', "\"\""),
            regclass_name(builder.schema, table),
            index.definition
        ))
        .await?;
    let built = builder.database.index_state(builder.schema, name).await?;
    Ok(built.is_some_and(|state| state.valid))
}
