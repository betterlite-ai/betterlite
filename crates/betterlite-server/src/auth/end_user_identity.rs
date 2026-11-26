use std::collections::BTreeMap;
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::http::{Request, header};
use betterlite_auth::auth_identity::records::EndUserIdentity;
use betterlite_auth::end_user_identity::resolve::{
    EndUserIdResolver, ResolveError, ResolveRequest,
};
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use betterlite_logging::end_user::extraction::{IdSettings, request_end_user_id};

use crate::auth::auth_identity::AuthContext;
use crate::auth::request_body_safety::request_object;
use crate::error_body::AuthenticationFailure;
use crate::state::ServerState;

mod tests;

// The resolver the proxy wired; this stage extracts the id and builds none of the lookup itself.
#[derive(Clone, Debug, Default)]
pub struct EndUserIdentityRule {
    pub resolver: Option<Arc<EndUserIdResolver>>,
}

#[derive(Clone, Debug)]
pub struct EndUserIdentityStageInput {
    pub context: AuthContext,
    pub raw_end_user_id: Option<String>,
    pub rule: Arc<EndUserIdentityRule>,
    pub database: bool,
}

fn rule_for_request(request: &Request<Body>) -> Arc<EndUserIdentityRule> {
    request
        .extensions()
        .get::<Arc<EndUserIdentityRule>>()
        .map_or_else(|| Arc::new(EndUserIdentityRule::default()), Arc::clone)
}

fn database_for_request(request: &Request<Body>) -> bool {
    request
        .extensions()
        .get::<Arc<ServerState>>()
        .is_some_and(|state| state.end_user_identity.database)
}

// B10050's request_end_user_id over the parsed body and the request headers
fn extracted_id(request: &Request<Body>, body: Option<&Bytes>) -> Option<String> {
    let slot = request
        .extensions()
        .get::<Arc<ServerState>>()
        .map(|state| &state.end_user_identity)?;
    let content_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let bytes = body.cloned().unwrap_or_default();
    let object = request_object(&bytes, content_type).unwrap_or_default();
    let settings = IdSettings {
        request_headers: &slot.request_headers,
        validate_end_user_id_in_db: slot.validate_end_user_id_in_db,
    };
    request_end_user_id(&object, Some(request.headers()), &settings)
}

pub(crate) fn stage_input(
    request: &Request<Body>,
    authenticated: Option<&AuthContext>,
    body: Option<&Bytes>,
) -> Result<EndUserIdentityStageInput, AuthenticationFailure> {
    let context = authenticated
        .cloned()
        .ok_or(AuthenticationFailure::Missing)?;
    Ok(EndUserIdentityStageInput {
        context,
        raw_end_user_id: extracted_id(request, body),
        rule: rule_for_request(request),
        database: database_for_request(request),
    })
}

// the validation is asynchronous, so the stage keeps its own input and the caller awaits the prefetch
pub(crate) async fn prefetch(
    stage: &mut EndUserIdentityStageInput,
) -> Result<(), Box<ProxyException>> {
    let resolved = match stage.rule.resolver.as_deref() {
        Some(resolver) if stage.database => resolver
            .resolve(&ResolveRequest {
                raw_end_user_id: stage.raw_end_user_id.as_deref(),
                key_end_user_budget_id: None,
            })
            .await
            .map_err(|error| Box::new(exception(&error)))?,
        // no database client is LiteLLM's prisma_client None, where the supplied id passes through
        Some(_) | None => stage.raw_end_user_id.clone(),
    };
    stage.context.principal.end_user = resolved.map(|id| EndUserIdentity { id });
    Ok(())
}

// LiteLLM lets a cache failure escape the auth path as an unhandled internal server error.
fn exception(error: &ResolveError) -> ProxyException {
    tracing::error!(error = %error, "the end-user id validation failed");
    ProxyException::new(ProxyExceptionInit {
        message: "Internal server error".to_owned(),
        error_type: "internal_server_error".to_owned(),
        param: None,
        code: ExceptionCode::Status(500_i64),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}
