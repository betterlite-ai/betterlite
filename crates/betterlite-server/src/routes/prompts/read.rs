#![cfg_attr(
    not(test),
    expect(dead_code, reason = "B8672 mounts the prompt read routes")
)]

mod access;
mod error;
mod rows;
mod specs;
mod tests;
mod wire;

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use betterlite_models::prompt_spec::PromptSpec;
use betterlite_prompts::registry::prompt_endpoint_contracts::table::PromptTableActions;
use betterlite_prompts::registry::prompt_registry::InMemoryPromptRegistry;
use betterlite_prompts::registry::prompt_registry_helpers::{
    get_base_prompt_id, get_version_number,
};
use serde::Deserialize;
use tokio::sync::RwLock;

use self::access::{PromptKeyView, info_access};
use self::error::PromptReadError;
use self::rows::{prompt_environments, resolve_spec, version_history_response};
use self::specs::{listable_prompts, not_found_detail, prompt_template};
use self::wire::{ListPromptsResponse, PromptInfoResponse, display_wire};
use crate::auth::auth_identity::AuthContext;
use crate::management_guards::caller::caller_has_admin_view;

const PROMPTS_LIST_PATH: &str = "/prompts/list";
const PROMPT_VERSIONS_PATH: &str = "/prompts/{prompt_id}/versions";
const PROMPT_INFO_PATH: &str = "/prompts/{prompt_id}/info";
const PROMPT_INFO_ALIAS_PATH: &str = "/prompts/{prompt_id}";
const VERSIONS_ADMIN_ONLY: &str = "Only proxy admins can view prompt versions";

pub(crate) struct PromptReadState<Table> {
    pub(crate) prompts: Arc<RwLock<InMemoryPromptRegistry>>,
    pub(crate) table: Option<Arc<Table>>,
}

impl<Table> Clone for PromptReadState<Table> {
    fn clone(&self) -> Self {
        Self {
            prompts: Arc::clone(&self.prompts),
            table: self.table.clone(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct EnvironmentQuery {
    environment: Option<String>,
}

pub(crate) fn routes<Table>(state: PromptReadState<Table>) -> Router
where
    Table: PromptTableActions + 'static,
{
    Router::new()
        .route(PROMPTS_LIST_PATH, get(list_prompts::<Table>))
        .route(PROMPT_VERSIONS_PATH, get(get_prompt_versions::<Table>))
        // LiteLLM serves GET /prompts/{prompt_id}
        .route(PROMPT_INFO_ALIAS_PATH, get(get_prompt_info::<Table>))
        .route(PROMPT_INFO_PATH, get(get_prompt_info::<Table>))
        .with_state(state)
}

async fn list_prompts<Table>(
    State(state): State<PromptReadState<Table>>,
    Extension(context): Extension<AuthContext>,
    Query(query): Query<EnvironmentQuery>,
) -> Result<Json<ListPromptsResponse>, PromptReadError<Table::Failure>>
where
    Table: PromptTableActions,
{
    // A key's metadata.prompts allowlist limits the list; a key without one lists nothing unless it has admin view
    let view =
        PromptKeyView::from_principal(&context.principal).map_err(PromptReadError::KeyMetadata)?;
    let allowlist = view
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.prompts.as_deref());
    if allowlist.is_none() && !caller_has_admin_view(&context.principal) {
        return Ok(Json(ListPromptsResponse {
            prompts: Vec::new(),
        }));
    }
    let visible: Vec<PromptSpec> = {
        let registry = state.prompts.read().await;
        registry.in_memory_prompts.values().cloned().collect()
    };
    let latest = listable_prompts(visible, allowlist, query.environment.as_deref());
    Ok(Json(ListPromptsResponse {
        prompts: latest.iter().map(|spec| display_wire(spec, None)).collect(),
    }))
}

// Non-admins get 403, and a prompt with no stored versions answers 404
async fn get_prompt_versions<Table>(
    State(state): State<PromptReadState<Table>>,
    Extension(context): Extension<AuthContext>,
    Path(prompt_id): Path<String>,
    Query(query): Query<EnvironmentQuery>,
) -> Result<Json<ListPromptsResponse>, PromptReadError<Table::Failure>>
where
    Table: PromptTableActions,
{
    if !caller_has_admin_view(&context.principal) {
        return Err(PromptReadError::Denied {
            status: StatusCode::FORBIDDEN,
            detail: VERSIONS_ADMIN_ONLY.to_owned(),
        });
    }
    Ok(Json(
        version_history_response(&state, &prompt_id, query.environment.as_deref()).await?,
    ))
}

async fn get_prompt_info<Table>(
    State(state): State<PromptReadState<Table>>,
    Extension(context): Extension<AuthContext>,
    Path(prompt_id): Path<String>,
    Query(query): Query<EnvironmentQuery>,
) -> Result<Json<PromptInfoResponse>, PromptReadError<Table::Failure>>
where
    Table: PromptTableActions,
{
    info_access::<Table::Failure>(&context.principal, &prompt_id)?;
    let base_prompt_id = get_base_prompt_id(&prompt_id);
    let wanted_environment = query.environment.as_deref();
    let environments = prompt_environments(state.table.as_deref(), &base_prompt_id).await?;
    let resolved = resolve_spec(&state, &base_prompt_id, &prompt_id, wanted_environment).await?;
    let Some(spec) = resolved else {
        return Err(PromptReadError::Denied {
            status: StatusCode::BAD_REQUEST,
            detail: not_found_detail(&prompt_id, wanted_environment),
        });
    };
    // The PromptTemplateBase comes from the spec's dotprompt_content; a registry prompt_manager template waits on B4052
    Ok(Json(PromptInfoResponse {
        prompt_spec: display_wire(&spec, Some(get_version_number(&spec.prompt_id))),
        raw_prompt_template: prompt_template(&spec, &base_prompt_id),
        environments,
    }))
}
