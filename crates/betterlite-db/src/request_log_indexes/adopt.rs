use crate::request_log_index_catalog::catalog::regclass_name;
use crate::request_log_index_catalog::definitions::RequestLogIndex;

use super::IndexBuilder;
use super::database::{IndexDatabase, IndexRef};

pub(crate) async fn adopt_equivalent<D>(
    builder: &IndexBuilder<'_, D>,
    target: IndexRef<'_>,
    index: RequestLogIndex<'_>,
) -> Result<bool, D::Error>
where
    D: IndexDatabase,
{
    let Some(equivalent) = builder
        .database
        .equivalent_indexes(target, index)
        .await?
        .into_iter()
        .find(|candidate| candidate.valid)
    else {
        return Ok(false);
    };
    tracing::info!(
        "Renaming the equivalent index {} on {} to {} instead of building a second one",
        equivalent.name,
        target.table,
        target.name
    );
    builder
        .database
        .execute(&rename_statement(
            target.schema,
            &equivalent.name,
            target.name,
        ))
        .await?;
    Ok(true)
}

fn rename_statement(schema: &str, from: &str, to: &str) -> String {
    format!(
        "ALTER INDEX {} RENAME TO \"{}\"",
        regclass_name(schema, from),
        to.replace('"', "\"\"")
    )
}
