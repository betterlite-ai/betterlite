use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse as _, Response};
use betterlite_auth::auth_resource_grants::search::authorize_search_tool;
use betterlite_auth::auth_resource_grants::vector_store::{
    deduped_vector_store_ids, permitted_vector_stores,
};
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_db::budget_permission_rows::LiteLLMObjectPermissionTable;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_error::envelope::{ErrorObject, ErrorResponse, WireField};
use betterlite_error::proxy_exception::ProxyException;

use crate::auth::auth_identity::{AuthContext, StageFailure, StageInput};
use crate::error_body::AuthenticationFailure;
use crate::state::ServerState;

mod tests;

// what the async auth service resolved before this stage runs: the caller's rows, the permission rows read fresh for them, and what the request asks to reach
#[derive(Debug, Default)]
pub struct ResourceGrantRule {
    pub key: Option<UserAPIKeyAuth>,
    pub team: Option<LiteLLMTeamTable>,
    pub object_permissions: BTreeMap<String, LiteLLMObjectPermissionTable>,
    pub search_tool_name: Option<String>,
    pub vector_store_ids: Vec<String>,
    pub rag_vector_store_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ResourceGrantStageInput {
    pub context: AuthContext,
    pub rule: Arc<ResourceGrantRule>,
    pub database: bool,
}

pub(crate) fn run_stage(
    stage: &ResourceGrantStageInput,
) -> Result<AuthContext, Box<ProxyException>> {
    if let (Some(name), Some(key)) = (
        stage.rule.search_tool_name.as_deref(),
        stage.rule.key.as_ref(),
    ) {
        authorize_search_tool(name, key, stage.rule.team.as_ref())?;
    }
    if stage.database {
        let rule = &stage.rule;
        let ids =
            deduped_vector_store_ids(&rule.vector_store_ids, rule.rag_vector_store_id.as_deref());
        if !ids.is_empty() {
            permitted_vector_stores(
                &ids,
                rule.key.as_ref(),
                rule.team.as_ref(),
                &rule.object_permissions,
            )?;
        }
    }
    Ok(stage.context.clone())
}

pub(crate) fn rule_for_request(request: &Request<Body>) -> Arc<ResourceGrantRule> {
    request
        .extensions()
        .get::<Arc<ResourceGrantRule>>()
        .map_or_else(|| Arc::new(ResourceGrantRule::default()), Arc::clone)
}

pub(crate) fn database_for_request(request: &Request<Body>) -> bool {
    request
        .extensions()
        .get::<Arc<ServerState>>()
        .is_some_and(|state| state.auth_resource_grants.database)
}

pub(crate) fn stage_input(
    request: &Request<Body>,
    authenticated: Option<&AuthContext>,
) -> Result<StageInput, StageFailure> {
    let context = authenticated
        .cloned()
        .ok_or(AuthenticationFailure::Missing)?;
    Ok(StageInput::AuthResourceGrants(Box::new(
        ResourceGrantStageInput {
            context,
            rule: rule_for_request(request),
            database: database_for_request(request),
        },
    )))
}

// ProxyException's response fields, with the code deciding the HTTP status as LiteLLM's handler does
pub(crate) fn denial_response(exception: &ProxyException) -> Response {
    let body = ErrorResponse {
        error: ErrorObject {
            message: exception.message.clone(),
            error_type: WireField::Text(exception.error_type.clone()),
            param: exception
                .param
                .clone()
                .map_or(WireField::Null, WireField::Text),
            code: WireField::Text(exception.code.clone()),
        },
    };
    let status = exception
        .code
        .parse::<u16>()
        .ok()
        .and_then(|value| StatusCode::from_u16(value).ok())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, Json(body)).into_response()
}
