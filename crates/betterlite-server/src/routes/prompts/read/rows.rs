use std::collections::BTreeSet;

use betterlite_models::prompt_spec::PromptSpec;
use betterlite_prompts::registry::prompt_endpoint_contracts::spec_conversion::create_versioned_prompt_spec;
use betterlite_prompts::registry::prompt_endpoint_contracts::table::{
    PromptColumn, PromptRowFilter, PromptRowOrder, PromptRowQuery, PromptTableActions,
};
use betterlite_prompts::registry::prompt_registry_helpers::{
    get_base_prompt_id, get_version_number,
};

use super::PromptReadState;
use super::error::PromptReadError;
use super::specs::{history_or_missing, history_response, in_memory_versions};
use super::wire::ListPromptsResponse;

fn rows_query(
    base_prompt_id: &str,
    environment: Option<&str>,
    version: Option<i64>,
    take: Option<i64>,
) -> PromptRowQuery {
    PromptRowQuery {
        filter: PromptRowFilter {
            id: None,
            prompt_id: Some(base_prompt_id.to_owned()),
            version,
            environment: environment.map(str::to_owned),
        },
        order: vec![PromptRowOrder {
            column: PromptColumn::Version,
            descending: true,
        }],
        take,
        distinct: Vec::new(),
    }
}

// LiteLLM serves history from the table, else the registry, and answers 404 when it is empty
pub(super) async fn version_history_response<Table>(
    state: &PromptReadState<Table>,
    prompt_id: &str,
    environment: Option<&str>,
) -> Result<ListPromptsResponse, PromptReadError<Table::Failure>>
where
    Table: PromptTableActions,
{
    let base_prompt_id = get_base_prompt_id(prompt_id);
    let versions = match state.table.as_deref() {
        None => in_memory_versions(state, &base_prompt_id, environment).await,
        Some(table) => table
            .find_many(rows_query(&base_prompt_id, environment, None, None))
            .await
            .map_err(PromptReadError::Table)?
            .iter()
            .map(create_versioned_prompt_spec)
            .collect::<Result<Vec<PromptSpec>, _>>()
            .map_err(PromptReadError::Row)?,
    };
    let versions = history_or_missing(versions, &base_prompt_id)?;
    Ok(history_response(&versions))
}

// LiteLLM lists the distinct environments a prompt is deployed to, sorted, with blank ones dropped
pub(super) async fn prompt_environments<Table>(
    table: Option<&Table>,
    base_prompt_id: &str,
) -> Result<Vec<String>, PromptReadError<Table::Failure>>
where
    Table: PromptTableActions,
{
    let Some(prompt_table) = table else {
        return Ok(Vec::new());
    };
    let rows = prompt_table
        .find_many(PromptRowQuery {
            filter: PromptRowFilter {
                id: None,
                prompt_id: Some(base_prompt_id.to_owned()),
                version: None,
                environment: None,
            },
            order: Vec::new(),
            take: None,
            distinct: vec![PromptColumn::Environment],
        })
        .await
        .map_err(PromptReadError::Table)?;
    let mut environments = BTreeSet::new();
    for row in rows {
        if !row.environment.is_empty() {
            environments.insert(row.environment);
        }
    }
    Ok(environments.into_iter().collect())
}

async fn spec_from_table<Table>(
    table: &Table,
    base_prompt_id: &str,
    requested_version: Option<i64>,
    environment: &str,
) -> Result<Option<PromptSpec>, PromptReadError<Table::Failure>>
where
    Table: PromptTableActions,
{
    let query = rows_query(
        base_prompt_id,
        Some(environment),
        requested_version,
        Some(1),
    );
    let rows = table
        .find_many(query)
        .await
        .map_err(PromptReadError::Table)?;
    rows.first()
        .map(create_versioned_prompt_spec)
        .transpose()
        .map_err(PromptReadError::Row)
}

pub(super) async fn resolve_spec<Table>(
    state: &PromptReadState<Table>,
    base_prompt_id: &str,
    prompt_id: &str,
    environment: Option<&str>,
) -> Result<Option<PromptSpec>, PromptReadError<Table::Failure>>
where
    Table: PromptTableActions,
{
    // A version suffix on the path picks that stored version; the bare id picks the latest
    let requested_version = if prompt_id == base_prompt_id {
        None
    } else {
        Some(get_version_number(prompt_id))
    };
    if let Some(wanted) = environment
        && let Some(table) = state.table.as_deref()
        && let Some(spec) =
            spec_from_table(table, base_prompt_id, requested_version, wanted).await?
    {
        return Ok(Some(spec));
    }
    let registry = state.prompts.read().await;
    Ok(registry.resolve_prompt_spec(prompt_id, requested_version, environment))
}
