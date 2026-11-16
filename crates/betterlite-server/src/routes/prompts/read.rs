use std::cmp::Reverse;
use std::collections::BTreeSet;
use std::error::Error as StdError;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_auth::resolved_key::auth::user_api_key_has_admin_view;
use betterlite_models::decisions::Json;
use betterlite_models::prompt_spec::{PromptInfo, PromptSpec};
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::dotprompt_templates::content::dotprompt_content_to_json;
use betterlite_prompts::registry::prompt_endpoint_contracts::spec_conversion::{
    PromptRowConversionError, create_versioned_prompt_spec, get_latest_prompt_versions,
};
use betterlite_prompts::registry::prompt_endpoint_contracts::table::{
    PromptColumn, PromptRowFilter, PromptRowOrder, PromptRowQuery, PromptTableActions,
};
use betterlite_prompts::registry::prompt_registry::InMemoryPromptRegistry;
use betterlite_prompts::registry::prompt_registry_helpers::{
    get_base_prompt_id, get_version_number,
};
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::RwLock;

use crate::auth::auth_identity::AuthContext;

const PROMPTS_LIST_PATH: &str = "/prompts/list";
const PROMPT_VERSIONS_PATH: &str = "/prompts/{prompt_id}/versions";
const PROMPT_INFO_PATH: &str = "/prompts/{prompt_id}/info";
const PROMPT_INFO_ALIAS_PATH: &str = "/prompts/{prompt_id}";
const MASTER_KEY_SUBJECT: &str = "master_key";
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

#[derive(Serialize)]
struct DetailBody {
    detail: String,
}

#[derive(Serialize)]
struct ListPromptsResponse {
    prompts: Vec<PromptSpecWire>,
}

#[derive(Serialize)]
struct PromptInfoResponse {
    prompt_spec: PromptSpecWire,
    raw_prompt_template: Option<PromptTemplateBase>,
    environments: Vec<String>,
}

#[derive(Serialize)]
struct PromptTemplateBase {
    litellm_prompt_id: String,
    content: String,
    metadata: Option<Object>,
}

#[derive(Serialize)]
struct PromptSpecWire {
    prompt_id: String,
    litellm_params: PromptLiteLLMParamsWire,
    prompt_info: PromptInfo,
    created_at: Option<DateTime<Utc>>,
    updated_at: Option<DateTime<Utc>>,
    version: Option<i64>,
    environment: String,
    created_by: Option<String>,
}

#[derive(Serialize)]
struct PromptLiteLLMParamsWire {
    prompt_id: Option<String>,
    prompt_integration: String,
    api_base: Option<String>,
    api_key: Option<String>,
    provider_specific_query_params: Option<IndexMap<String, Json>>,
    ignore_prompt_manager_model: bool,
    #[serde(rename = "ignore_prompt_manager_optional_params")]
    ignore_optional_params: bool,
    dotprompt_content: Option<String>,
    #[serde(flatten)]
    extra: IndexMap<String, Json>,
}

impl From<&PromptSpec> for PromptSpecWire {
    fn from(spec: &PromptSpec) -> Self {
        let params = &spec.litellm_params;
        Self {
            prompt_id: spec.prompt_id.clone(),
            litellm_params: PromptLiteLLMParamsWire {
                prompt_id: params.prompt_id.clone(),
                prompt_integration: params.prompt_integration.clone(),
                api_base: params.api_base.clone(),
                // LiteLLM repeats a configured prompt key in plaintext; betterlite never sends a secret back
                api_key: params
                    .api_key
                    .as_ref()
                    .map(|_secret| crate::cache_settings::REDACTED_VALUE.to_owned()),
                provider_specific_query_params: params.provider_specific_query_params.clone(),
                ignore_prompt_manager_model: params.ignore_model,
                ignore_optional_params: params.ignore_optional_params,
                dotprompt_content: params.dotprompt_content.clone(),
                extra: params.extra.clone(),
            },
            prompt_info: spec.prompt_info.clone(),
            created_at: spec.created_at,
            updated_at: spec.updated_at,
            version: spec.version,
            environment: spec.environment.clone(),
            created_by: spec.created_by.clone(),
        }
    }
}

// LiteLLM answers display copies: the base prompt id, and only the version a history row names
fn display_wire(spec: &PromptSpec, version: Option<i64>) -> PromptSpecWire {
    let mut wire = PromptSpecWire::from(spec);
    wire.prompt_id = get_base_prompt_id(&wire.prompt_id);
    wire.version = version;
    wire
}

// LiteLLM's user_api_key_has_admin_view, and the master key, which carries no key record to read a role from
fn has_admin_view(principal: &Principal) -> bool {
    principal.subject == MASTER_KEY_SUBJECT
        || principal
            .source_key
            .0
            .as_ref()
            .is_some_and(user_api_key_has_admin_view)
        || principal.roles.contains(&Role::PlatformAdmin)
        || principal.roles.contains(&Role::PlatformViewer)
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PromptKeyView {
    metadata: Option<PromptKeyMetadata>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PromptKeyMetadata {
    prompts: Option<Vec<String>>,
}

impl PromptKeyView {
    fn from_principal(principal: &Principal) -> Result<Self, serde_json::Error> {
        let Some(key) = principal.source_key.0.as_ref() else {
            return Ok(Self::default());
        };
        serde_json::from_value(serde_json::to_value(key)?)
    }

    fn allowlist(&self) -> Option<&[String]> {
        self.metadata.as_ref()?.prompts.as_deref()
    }
}

// LiteLLM lists a prompt when the key's allowlist names it by exact id or by the base id without its version
fn names_prompt(allowlist: &[String], prompt_id: &str) -> bool {
    let base = get_base_prompt_id(prompt_id);
    allowlist
        .iter()
        .any(|entry| entry == prompt_id || *entry == base)
}

fn not_found_detail(prompt_id: &str, environment: Option<&str>) -> String {
    let Some(wanted) = environment else {
        return format!("Prompt {prompt_id} not found");
    };
    format!("Prompt {prompt_id} not found in environment {wanted}")
}

fn filter_environment(prompts: Vec<PromptSpec>, environment: Option<&str>) -> Vec<PromptSpec> {
    let Some(wanted) = environment else {
        return prompts;
    };
    prompts
        .into_iter()
        .filter(|spec| spec.environment == wanted)
        .collect()
}

async fn prompt_specs<Table>(state: &PromptReadState<Table>) -> Vec<PromptSpec> {
    let registry = state.prompts.read().await;
    registry.in_memory_prompts.values().cloned().collect()
}

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

// LiteLLM serves history from the registry, newest first, when the proxy has no database
async fn in_memory_versions<Table>(
    state: &PromptReadState<Table>,
    base_prompt_id: &str,
    environment: Option<&str>,
) -> Vec<PromptSpec> {
    let mut versions: Vec<PromptSpec> = {
        let registry = state.prompts.read().await;
        registry
            .in_memory_prompts
            .values()
            .filter(|spec| get_base_prompt_id(&spec.prompt_id) == base_prompt_id)
            .filter(|spec| environment.is_none_or(|wanted| spec.environment == wanted))
            .cloned()
            .collect()
    };
    versions.sort_by_key(|spec| Reverse(get_version_number(&spec.prompt_id)));
    versions
}

async fn prompt_version_history<Table>(
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
async fn prompt_environments<Table>(
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

// A dotprompt parse failure yields no template, and DotpromptManager.prompt_manager's templates need a public accessor (B4052)
fn prompt_template(spec: &PromptSpec, base_prompt_id: &str) -> Option<PromptTemplateBase> {
    let parsed =
        dotprompt_content_to_json(spec.litellm_params.dotprompt_content.as_deref()?).ok()?;
    Some(PromptTemplateBase {
        litellm_prompt_id: base_prompt_id.to_owned(),
        content: parsed.get("content")?.as_str()?.to_owned(),
        metadata: parsed.get("metadata")?.as_object().cloned(),
    })
}

async fn resolve_spec<Table>(
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

#[derive(Debug, Error)]
enum PromptReadError<TableError>
where
    TableError: StdError + 'static,
{
    #[error("{detail}")]
    Denied { status: StatusCode, detail: String },
    #[error("the prompt key's metadata.prompts is not a list of strings")]
    KeyMetadata(#[source] serde_json::Error),
    #[error("a stored prompt row is not a valid prompt")]
    Row(#[source] PromptRowConversionError),
    #[error("the prompt table query failed")]
    Table(#[source] TableError),
}

impl<TableError> IntoResponse for PromptReadError<TableError>
where
    TableError: StdError + 'static,
{
    fn into_response(self) -> Response {
        let status = match &self {
            Self::Denied { status, .. } => *status,
            Self::KeyMetadata(_) => StatusCode::BAD_REQUEST,
            Self::Row(_) | Self::Table(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = Json(DetailBody {
            detail: self.to_string(),
        });
        (status, body).into_response()
    }
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
    let view =
        PromptKeyView::from_principal(&context.principal).map_err(PromptReadError::KeyMetadata)?;
    let allowlist = view.allowlist();
    if allowlist.is_none() && !has_admin_view(&context.principal) {
        return Ok(Json(ListPromptsResponse {
            prompts: Vec::new(),
        }));
    }
    let mut visible = prompt_specs(&state).await;
    if let Some(allowed) = allowlist {
        visible.retain(|spec| names_prompt(allowed, &spec.prompt_id));
    }
    let latest =
        get_latest_prompt_versions(filter_environment(visible, query.environment.as_deref()));
    Ok(Json(ListPromptsResponse {
        prompts: latest.iter().map(|spec| display_wire(spec, None)).collect(),
    }))
}

async fn get_prompt_versions<Table>(
    State(state): State<PromptReadState<Table>>,
    Extension(context): Extension<AuthContext>,
    Path(prompt_id): Path<String>,
    Query(query): Query<EnvironmentQuery>,
) -> Result<Json<ListPromptsResponse>, PromptReadError<Table::Failure>>
where
    Table: PromptTableActions,
{
    if !has_admin_view(&context.principal) {
        return Err(PromptReadError::Denied {
            status: StatusCode::FORBIDDEN,
            detail: VERSIONS_ADMIN_ONLY.to_owned(),
        });
    }
    let base_prompt_id = get_base_prompt_id(&prompt_id);
    let versions =
        prompt_version_history(&state, &base_prompt_id, query.environment.as_deref()).await?;
    // LiteLLM answers 404 when the prompt has no stored versions
    if versions.is_empty() {
        return Err(PromptReadError::Denied {
            status: StatusCode::NOT_FOUND,
            detail: format!("No versions found for prompt ID {base_prompt_id}"),
        });
    }
    Ok(Json(ListPromptsResponse {
        prompts: versions
            .iter()
            .map(|spec| display_wire(spec, Some(get_version_number(&spec.prompt_id))))
            .collect(),
    }))
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
    let view =
        PromptKeyView::from_principal(&context.principal).map_err(PromptReadError::KeyMetadata)?;
    let allowlist = view.allowlist();
    if let Some(allowed) = allowlist
        && !allowed.iter().any(|entry| entry == &prompt_id)
    {
        return Err(PromptReadError::Denied {
            status: StatusCode::BAD_REQUEST,
            detail: format!("Prompt {prompt_id} not found"),
        });
    }
    if !has_admin_view(&context.principal) {
        return Err(PromptReadError::Denied {
            status: StatusCode::FORBIDDEN,
            detail: "You are not authorized to access this prompt".to_owned(),
        });
    }
    let base_prompt_id = get_base_prompt_id(&prompt_id);
    let environment = query.environment.as_deref();
    let environments = prompt_environments(&state, &base_prompt_id).await?;
    let requested_version = if prompt_id == base_prompt_id {
        None
    } else {
        Some(get_version_number(&prompt_id))
    };
    let resolved = resolve_spec(
        &state,
        &base_prompt_id,
        &prompt_id,
        requested_version,
        environment,
    )
    .await?;
    let Some(spec) = resolved else {
        return Err(PromptReadError::Denied {
            status: StatusCode::BAD_REQUEST,
            detail: not_found_detail(&prompt_id, environment),
        });
    };
    Ok(Json(PromptInfoResponse {
        prompt_spec: display_wire(&spec, Some(get_version_number(&spec.prompt_id))),
        raw_prompt_template: prompt_template(&spec, &base_prompt_id),
        environments,
    }))
}

mod tests;
