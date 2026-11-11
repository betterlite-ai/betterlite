use crate::request_log_index_catalog::catalog::regclass_name;
use crate::request_log_index_catalog::definitions::RequestLogIndex;

use super::IndexBuilder;
use super::copies::equivalent_copies;
use super::database::IndexDatabase;

pub(crate) async fn adopt_equivalent<D>(
    builder: &IndexBuilder<'_, D>,
    table: &str,
    name: &str,
    index: RequestLogIndex<'_>,
) -> Result<bool, D::Error>
where
    D: IndexDatabase,
{
    let Some(equivalent) = equivalent_copies(builder, table, name, index)
        .await?
        .into_iter()
        .find(|candidate| candidate.valid)
    else {
        return Ok(false);
    };
    tracing::info!(
        "Renaming the equivalent index {} on {table} to {name} instead of building a second one",
        equivalent.name
    );
    builder
        .database
        .execute(&rename_statement(builder.schema, &equivalent.name, name))
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
