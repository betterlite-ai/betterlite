use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::Arc;

use async_trait::async_trait;
use axum::Extension;
use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::Uri;
use axum::routing::post;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_logging::otel::service::management::ManagementPayload;
use betterlite_logging::otel::v2_callback::live_span::LiveSpan;
use betterlite_models::model_registration::JsonObject;
use chrono::{DateTime, FixedOffset, Utc};

use crate::alerting::model_alerts::key_event::VirtualKeyEvent;
use crate::auth::auth_identity::AuthContext;
use crate::failure_hook::ports::Clock;
use crate::management_effects::eviction::ManagementCacheEviction;
use crate::management_effects::wrapper::{
    ManagementCall, ManagementDeps, management_endpoint_wrapper,
};
use crate::management_payload::alerts::ManagementAlertTransport;
use crate::management_payload::otel_span::{ManagementSpanDeps, ManagementSpanTransport};
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
    evaluate(&state, caller, &body, regulation).await.map(Json)
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
    let default_auth = resolve_default_auth(source_key.as_ref())?;
    let (_, function_name) = regulation.names();
    let limits = ApiLimits::default();
    let cache = NoCache;
    let deps = ManagementDeps {
        alert: Option::<&NoAlert>::None,
        span: ManagementSpanDeps {
            transport: Option::<&NoSpan>::None,
            otel_v2_enabled: false,
            body_settings: &state.server.config.sections.request_body,
            limits: &limits,
        },
        cache: &cache,
        default_auth: &default_auth,
        clock: Arc::new(SystemClock),
    };
    let call = ManagementCall {
        function_name,
        request_kwargs: JsonObject::new(),
        data: None,
        user_api_key_dict: source_key.as_ref(),
        parent_otel_span: None,
        http_request: None,
    };
    management_endpoint_wrapper::<ComplianceResponse, ComplianceError, _, NoAlert, NoSpan, NoCache>(
        call,
        deps,
        async move { Ok(ComplianceChecker::new(&request).response(regulation)) },
    )
    .await
}

// LiteLLM's kwargs carry the authenticated key; the wrapper falls back to UserAPIKeyAuth() without one.
fn resolve_default_auth(
    source: Option<&UserAPIKeyAuth>,
) -> Result<UserAPIKeyAuth, ComplianceError> {
    let Some(auth) = source else {
        return UserAPIKeyAuth::construct(AuthInput::Dict(BTreeMap::new()))
            .map_err(ComplianceError::DefaultAuth);
    };
    Ok(auth.clone())
}

struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<FixedOffset> {
        let moment = Utc::now();
        moment.fixed_offset()
    }
}

// The wrapper's effects are wired, and a compliance call has no alert name, no parent span and no cached id.
struct NoAlert;

#[async_trait]
impl ManagementAlertTransport for NoAlert {
    type Error = Infallible;

    async fn send_management_event_alert(
        &self,
        _key_event: &VirtualKeyEvent,
        _alert_type: AlertType,
        _event_name: &str,
    ) -> Result<(), Infallible> {
        Ok(())
    }
}

struct NoSpan;

#[async_trait]
impl ManagementSpanTransport for NoSpan {
    type Error = Infallible;

    async fn success_hook(
        &self,
        _payload: &ManagementPayload,
        _parent_otel_span: Option<&LiveSpan>,
    ) -> Result<(), Infallible> {
        Ok(())
    }

    async fn failure_hook(
        &self,
        _payload: &ManagementPayload,
        _parent_otel_span: Option<&LiveSpan>,
    ) -> Result<(), Infallible> {
        Ok(())
    }
}

struct NoCache;

impl ManagementCacheEviction for NoCache {
    type Error = Infallible;

    fn delete_cache(&self, _key: &str) -> Result<(), Infallible> {
        Ok(())
    }
}
