use axum::Extension;
use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::Uri;
use axum::routing::post;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_models::model_registration::JsonObject;

use crate::auth::auth_identity::AuthContext;
use crate::management_effects::wrapper::{
    ManagementCall, NoEffects, default_management_auth, management_endpoint_wrapper, no_effect_deps,
};
use crate::state::ServerState;

use super::checker::{ComplianceChecker, Regulation};
use super::decode::decode_compliance_request;
use super::error::ComplianceError;
use super::response::ComplianceResponse;

const EU_AI_ACT_PATH: &str = "/compliance/eu-ai-act";
const GDPR_PATH: &str = "/compliance/gdpr";

#[derive(Clone)]
struct RouteState {
    server: Arc<ServerState>,
}

pub(crate) fn routes(state: &Arc<ServerState>) -> Router {
    Router::new()
        .route(EU_AI_ACT_PATH, post(check))
        .route(GDPR_PATH, post(check))
        .with_state(RouteState {
            server: Arc::clone(state),
        })
}

// The root-path layer strips its prefix from the routed URI, so the matched Uri picks the regulation.
async fn check(
    State(state): State<RouteState>,
    uri: Uri,
    caller: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Result<Json<ComplianceResponse>, ComplianceError> {
    let regulation = if uri.path() == GDPR_PATH {
        Regulation::Gdpr
    } else {
        Regulation::EuAiAct
    };
    Box::pin(evaluate(&state, caller, &body, regulation))
        .await
        .map(Json)
}

// LiteLLM validates the body in a FastAPI dependency, so a 422 never reaches the management wrapper.
async fn evaluate(
    state: &RouteState,
    caller: Option<Extension<AuthContext>>,
    body: &[u8],
    regulation: Regulation,
) -> Result<ComplianceResponse, ComplianceError> {
    let request = decode_compliance_request(body)?;
    let source_key = caller
        .as_ref()
        .and_then(|extension| extension.0.principal.source_key.0.clone());
    let default_auth =
        default_management_auth(source_key.as_ref()).map_err(ComplianceError::DefaultAuth)?;
    let (_, function_name) = regulation.names();
    let limits = ApiLimits::default();
    let effects = NoEffects;
    let body_settings = &state.server.config.sections.request_body;
    let deps = no_effect_deps(body_settings, &limits, &effects, &default_auth);
    let call = ManagementCall {
        function_name,
        request_kwargs: JsonObject::new(),
        data: None,
        user_api_key_dict: source_key.as_ref(),
        parent_otel_span: None,
        http_request: None,
    };
    management_endpoint_wrapper::<
        ComplianceResponse,
        ComplianceError,
        _,
        NoEffects,
        NoEffects,
        NoEffects,
    >(call, deps, async move {
        Ok(ComplianceChecker::new(&request).response(regulation))
    })
    .await
}
