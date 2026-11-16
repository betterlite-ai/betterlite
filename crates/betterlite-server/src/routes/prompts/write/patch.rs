use betterlite_prompts::registry::prompt_endpoint_contracts::records::PatchPromptRequest;
use betterlite_prompts::registry::prompt_endpoint_contracts::rows::{PromptRow, PromptRowJson};
use betterlite_prompts::registry::prompt_endpoint_contracts::spec_conversion::create_versioned_prompt_spec;
use betterlite_prompts::registry::prompt_endpoint_contracts::table::{
    PromptColumn, PromptRowFilter, PromptRowOrder, PromptRowQuery, PromptRowUpdate,
    PromptTableActions,
};

use super::WriteResult;
use super::error::PromptWriteError;
use super::state::Writer;

pub(super) async fn find_patch_target<Table>(
    table: &Table,
    base_prompt_id: &str,
    requested_version: Option<i64>,
    environment: &str,
) -> WriteResult<PromptRow, Table>
where
    Table: PromptTableActions,
{
    let query = PromptRowQuery {
        filter: PromptRowFilter {
            id: None,
            prompt_id: Some(base_prompt_id.to_owned()),
            version: requested_version,
            environment: Some(environment.to_owned()),
        },
        order: vec![PromptRowOrder {
            column: PromptColumn::Version,
            descending: true,
        }],
        take: Some(1),
        distinct: Vec::new(),
    };
    let rows = table
        .find_many(query)
        .await
        .map_err(|source| PromptWriteError::Table { source })?;
    rows.into_iter()
        .next()
        .ok_or_else(|| PromptWriteError::NotFound {
            detail: format!(
                "Prompt with ID {base_prompt_id} not found in environment {environment}"
            ),
        })
}

// A field the request leaves out keeps the stored value, and the primary key targets exactly one row
pub(super) async fn patch_row<Table>(
    writer: &Writer<'_, Table>,
    target_row: &PromptRow,
    request: PatchPromptRequest,
) -> WriteResult<PromptRow, Table>
where
    Table: PromptTableActions,
{
    let current_spec = create_versioned_prompt_spec(target_row)?;
    let litellm_params = request
        .litellm_params
        .unwrap_or(current_spec.litellm_params);
    let prompt_info = request.prompt_info.unwrap_or(current_spec.prompt_info);
    let update = PromptRowUpdate {
        litellm_params: PromptRowJson::Text(serde_json::to_string(&litellm_params)?),
        prompt_info: PromptRowJson::Text(serde_json::to_string(&prompt_info)?),
        created_by: writer.user_id.clone(),
    };
    let filter = PromptRowFilter {
        id: Some(target_row.id.clone()),
        prompt_id: None,
        version: None,
        environment: None,
    };
    let updated = writer
        .table
        .update(filter, update)
        .await
        .map_err(|source| PromptWriteError::Table { source })?;
    updated.ok_or_else(|| PromptWriteError::NotFound {
        detail: format!(
            "Prompt with ID {} not found in environment {}",
            target_row.prompt_id, target_row.environment
        ),
    })
}
