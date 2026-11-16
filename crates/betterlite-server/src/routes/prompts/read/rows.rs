use std::collections::BTreeSet;

use betterlite_models::prompt_spec::PromptSpec;
use betterlite_prompts::registry::prompt_endpoint_contracts::spec_conversion::create_versioned_prompt_spec;
use betterlite_prompts::registry::prompt_endpoint_contracts::table::{
    PromptColumn, PromptRowFilter, PromptRowOrder, PromptRowQuery, PromptTableActions,
};

use super::PromptReadState;
use super::error::PromptReadError;
use super::specs::in_memory_versions;

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

fn environment_query(base_prompt_id: &str) -> PromptRowQuery {
    PromptRowQuery {
        filter: PromptRowFilter {
            id: None,
            prompt_id: Some(base_prompt_id.to_owned()),
            version: None,
            environment: None,
        },
        order: Vec::new(),
        take: None,
        distinct: vec![PromptColumn::Environment],
    }
}

pub(super) async fn prompt_version_history<Table>(
    state: &PromptReadState<Table>,
    base_prompt_id: &str,
    environment: Option<&str>,
) -> Result<Vec<PromptSpec>, PromptReadError<Table::Failure>>
where
    Table: PromptTableActions,
{
    let Some(table) = state.table.as_ref() else {
        return Ok(in_memory_versions(state, base_prompt_id, environment).await);
    };
    let rows = table
        .find_many(rows_query(base_prompt_id, environment, None, None))
        .await
        .map_err(PromptReadError::Table)?;
    rows.iter()
        .map(create_versioned_prompt_spec)
        .collect::<Result<Vec<PromptSpec>, _>>()
        .map_err(PromptReadError::Row)
}

// LiteLLM lists the distinct environments a prompt is deployed to, sorted, with blank ones dropped
pub(super) async fn prompt_environments<Table>(
    state: &PromptReadState<Table>,
    base_prompt_id: &str,
) -> Result<Vec<String>, PromptReadError<Table::Failure>>
where
    Table: PromptTableActions,
{
    let Some(table) = state.table.as_ref() else {
        return Ok(Vec::new());
    };
    let rows = table
        .find_many(environment_query(base_prompt_id))
        .await
        .map_err(PromptReadError::Table)?;
    let environments: BTreeSet<String> = rows
        .into_iter()
        .map(|row| row.environment)
        .filter(|environment| !environment.is_empty())
        .collect();
    Ok(environments.into_iter().collect())
}

pub(super) async fn resolve_spec<Table>(
    state: &PromptReadState<Table>,
    base_prompt_id: &str,
    prompt_id: &str,
    requested_version: Option<i64>,
    environment: Option<&str>,
) -> Result<Option<PromptSpec>, PromptReadError<Table::Failure>>
where
    Table: PromptTableActions,
{
    if let (Some(wanted), Some(table)) = (environment, state.table.as_ref()) {
        let rows = table
            .find_many(rows_query(
                base_prompt_id,
                Some(wanted),
                requested_version,
                Some(1),
            ))
            .await
            .map_err(PromptReadError::Table)?;
        if let Some(row) = rows.first() {
            let spec = create_versioned_prompt_spec(row).map_err(PromptReadError::Row)?;
            return Ok(Some(spec));
        }
    }
    let registry = state.prompts.read().await;
    Ok(registry.resolve_prompt_spec(prompt_id, requested_version, environment))
}
