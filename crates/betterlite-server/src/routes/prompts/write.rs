#![cfg_attr(
    not(test),
    expect(dead_code, reason = "B8672 mounts the prompt write routes")
)]

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::routing::{post, put};
use betterlite_models::prompt_spec::PromptSpec;
use betterlite_prompts::registry::mutation::prompt_registry_mutation::methods::PromptRegistryMutation as _;
use betterlite_prompts::registry::prompt_endpoint_contracts::records::{
    Prompt, is_ambiguous_keyed_prompt_data,
};
use betterlite_prompts::registry::prompt_endpoint_contracts::spec_conversion::create_versioned_prompt_spec;
use betterlite_prompts::registry::prompt_endpoint_contracts::table::{
    PromptTableActions, get_next_version_for_prompt,
};
use betterlite_prompts::registry::prompt_registry_helpers::prompt_environment_or_default;

use self::create::create_prompt;
use self::delete::delete_prompt;
use self::error::PromptWriteError;
use self::patch::patch_prompt;
use self::state::{PromptWriteState, Writer};
use self::update::update_prompt;
use self::wire::{decode_body, new_row};
use crate::auth::auth_identity::AuthContext;

mod create;
mod delete;
mod error;
mod patch;
pub(crate) mod state;
mod tests;
mod update;
mod wire;

type WriteResult<Value, Table> =
    Result<Value, PromptWriteError<<Table as PromptTableActions>::Failure>>;

// every handler first answers 403 for a non-admin and 500 for a missing database, as the guard does
pub(crate) fn routes<Table>(state: Arc<PromptWriteState<Table>>) -> Router
where
    Table: PromptTableActions + 'static,
{
    Router::new()
        .route("/prompts", post(create_prompt::<Table>))
        .route(
            "/prompts/{prompt_id}",
            put(update_prompt::<Table>)
                .patch(patch_prompt::<Table>)
                .delete(delete_prompt::<Table>),
        )
        .with_state(state)
}

// POST and PUT decode the body, answer 403 and 500, then reject ambiguous data, in LiteLLM's order
fn prompt_write_request<'state, Table>(
    state: &'state PromptWriteState<Table>,
    context: &AuthContext,
    body: &[u8],
    action: &'static str,
) -> WriteResult<(Prompt, Writer<'state, Table>), Table>
where
    Table: PromptTableActions,
{
    let request: Prompt = decode_body(body)?;
    let writer = state.require_admin_and_db(context, action)?;
    if is_ambiguous_keyed_prompt_data(&request.litellm_params) {
        return Err(PromptWriteError::Ambiguous);
    }
    Ok((request, writer))
}

async fn store_new_version<Table>(
    state: &PromptWriteState<Table>,
    writer: &Writer<'_, Table>,
    prompt_id: &str,
    request: &Prompt,
) -> WriteResult<PromptSpec, Table>
where
    Table: PromptTableActions,
{
    let environment = prompt_environment_or_default(
        request
            .prompt_info
            .as_ref()
            .map(|info| info.environment.as_str()),
    );
    let version = get_next_version_for_prompt(writer.table, prompt_id, Some(&environment))
        .await
        .map_err(|source| PromptWriteError::Version { source })?;
    let row_data = new_row(
        prompt_id,
        version,
        environment,
        writer.user_id.clone(),
        request,
    )?;
    let row = writer
        .table
        .create(row_data)
        .await
        .map_err(|source| PromptWriteError::Table { source })?;
    let spec = create_versioned_prompt_spec(&row)?;
    // the database row stays when the registry cannot initialize it, and the caller sees a 500
    let mut registry = state.registry.lock().await;
    let mut callbacks = state.callbacks.lock().await;
    registry
        .initialize_prompt(&spec, None, &mut *callbacks)
        .map_err(PromptWriteError::from)
}

async fn reject_config_prompt<Table>(
    state: &PromptWriteState<Table>,
    base_prompt_id: &str,
    action: &'static str,
) -> WriteResult<(), Table>
where
    Table: PromptTableActions,
{
    let is_config = {
        let registry = state.registry.lock().await;
        registry.has_config_prompt(base_prompt_id)
    };
    if is_config {
        return Err(PromptWriteError::ConfigPrompt { action });
    }
    Ok(())
}
