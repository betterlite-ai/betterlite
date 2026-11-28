use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Extension, State};
use axum::http::HeaderMap;
use axum::routing::post;
use betterlite_auth::request_quota_view::RequestQuotaViewExt as _;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_config::sections::mcp_env_values::PayloadRules;
use betterlite_config::sections::request_body::Settings as RequestBodySettings;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use betterlite_logging::logging_metadata::exception::ExceptionFacts;
use betterlite_mcp::registry::public_list::PublicMcpServers as _;
use betterlite_mcp::registry::resolution::ServerAccessCheck;
use betterlite_mcp::registry::servers::SharedRegistry;
use betterlite_mcp::server_payload::names::validate_and_normalize_payload;
use betterlite_mcp::server_payload::request::NewMCPServerRequest;
use betterlite_mcp::server_payload::rules::stamp_omitted_oauth2_flow;
use betterlite_models::model_registration::JsonObject;
use betterlite_prompts::arguments::json::Json as WireJson;
use chrono::Utc;

use self::effects::NoManagementEffects;
use crate::auth::auth_identity::AuthContext;
use crate::failure_hook::ports::Clock as _;
use crate::management_effects::wrapper::{
    ManagementCall, ManagementDeps, ManagementFailure, management_endpoint_wrapper,
};
use crate::management_payload::otel_span::ManagementSpanDeps;
use crate::mcp::admin_credential_view::redact_mcp_credentials;
use crate::mcp::management_visibility::{
    // ast-grep-ignore: no-long-names
    inherit_credentials_from_existing_server as inherit_credentials,
    user_is_full_admin,
};
use crate::mcp::oauth_sessions::record::build_temporary_record;
use crate::mcp::oauth_sessions::redis_copy::write_redis_copy;
use crate::mcp::oauth_sessions::store::SessionStore;
use crate::mcp::oauth_sessions::{DEFAULT_CREATED_BY, SessionError};
// ast-grep-ignore: no-long-names
use crate::mcp::oauth_sessions::TEMPORARY_MCP_SERVER_TTL_SECONDS as SESSION_TTL;

mod effects;
mod tests;

// LiteLLM mounts the router under /v1/mcp, so the route's full path is this one
pub const SESSION_PATH: &str = "/v1/mcp/server/oauth/session";

const FUNCTION_NAME: &str = "add_session_mcp_server";

// the 403 body for every role but PROXY_ADMIN, PROXY_ADMIN_VIEW_ONLY included
pub const FORBIDDEN_MESSAGE: &str = "User does not have permission to create temporary mcp servers. You can only create temporary mcp servers if you are a PROXY_ADMIN.";

// what B8702 merges into the app behind with_key_auth; the store and registry are the caller's
#[derive(Clone)]
pub struct SessionRoute<Access>
where
    Access: ServerAccessCheck,
{
    pub store: Arc<SessionStore<Access>>,
    pub registry: SharedRegistry,
    pub rules: PayloadRules,
}

// the one-place effects of this route: no alert, no span transport and no cached key to evict
#[derive(Default)]
struct SessionEffects {
    limits: ApiLimits,
    body_settings: RequestBodySettings,
}

impl<Access> SessionRoute<Access>
where
    Access: ServerAccessCheck + 'static,
{
    #[must_use]
    pub fn router(self) -> Router {
        Router::new()
            .route(SESSION_PATH, post(session_endpoint::<Access>))
            .with_state(Arc::new(self))
    }

    // LiteLLM's add_session_mcp_server: the steps its handler runs, in the order it runs them
    async fn create_session(
        &self,
        body: &[u8],
        auth: &UserAPIKeyAuth,
    ) -> Result<McpServerTable, SessionError<Access::Error>> {
        let (payload, created_by) = session_payload(self, body, auth).await?;
        let server_id = {
            let registry = self.registry.read().await;
            self.store.resolve_session_id(&payload, &registry).await?
        };
        let now = self.store.clock.now().with_timezone(&Utc);
        let record = build_temporary_record(&payload, Some(created_by.as_str()), &server_id, now)?;
        cache_session(self, &record, &payload, &server_id, &created_by).await?;
        let registry = self.registry.read().await;
        let redacted = redact_mcp_credentials(&record, &registry, registry.public_mcp_servers())?;
        Ok(redacted)
    }
}

async fn session_endpoint<Access>(
    State(route): State<Arc<SessionRoute<Access>>>,
    caller: Option<Extension<AuthContext>>,
    _headers: HeaderMap,
    body: Bytes,
) -> Result<Json<McpServerTable>, SessionError<Access::Error>>
where
    Access: ServerAccessCheck + 'static,
{
    // litellm-changed-by is declared and never read, like LiteLLM's Header(None) parameter
    let auth = authenticate::<Access::Error>(caller)?;
    let record = Box::pin(run_session(&route, &auth, &body)).await?;
    Ok(Json(record))
}

// the wrapper's call and its one-place effects: no alert, no span transport and no cached key
async fn run_session<Access>(
    route: &SessionRoute<Access>,
    auth: &UserAPIKeyAuth,
    body: &[u8],
) -> Result<McpServerTable, SessionError<Access::Error>>
where
    Access: ServerAccessCheck + 'static,
{
    let effects = SessionEffects::default();
    let cache = NoManagementEffects;
    let deps = ManagementDeps {
        alert: Option::<&NoManagementEffects>::None,
        span: ManagementSpanDeps {
            transport: Option::<&NoManagementEffects>::None,
            otel_v2_enabled: false,
            body_settings: &effects.body_settings,
            limits: &effects.limits,
        },
        cache: &cache,
        default_auth: auth,
        clock: Arc::clone(&route.store.clock),
    };
    let handler = async move {
        route
            .create_session(body, auth)
            .await
            .map_err(management_failure)
    };
    Box::pin(management_endpoint_wrapper::<
        McpServerTable,
        SessionError<Access::Error>,
        _,
        NoManagementEffects,
        NoManagementEffects,
        NoManagementEffects,
    >(session_call(auth), deps, handler))
    .await
}

fn session_call(auth: &UserAPIKeyAuth) -> ManagementCall<'_> {
    ManagementCall {
        function_name: FUNCTION_NAME,
        request_kwargs: JsonObject::new(),
        data: None,
        user_api_key_dict: Some(auth),
        parent_otel_span: None,
        http_request: None,
    }
}

fn authenticate<Failure>(
    caller: Option<Extension<AuthContext>>,
) -> Result<UserAPIKeyAuth, SessionError<Failure>>
where
    Failure: StdError + Send + Sync + 'static,
{
    caller
        .and_then(|Extension(context)| context.principal.source_key.0)
        .ok_or(SessionError::Unauthenticated)
}

// steps (a)-(e): validate, stamp, the PROXY_ADMIN gate, created_by and the credential inheritance
async fn session_payload<Access>(
    route: &SessionRoute<Access>,
    body: &[u8],
    auth: &UserAPIKeyAuth,
) -> Result<(NewMCPServerRequest, String), SessionError<Access::Error>>
where
    Access: ServerAccessCheck,
{
    let value: WireJson = serde_json::from_slice(body).map_err(SessionError::Body)?;
    let mut payload = NewMCPServerRequest::from_value(&value, &route.rules.stdio)?;
    validate_and_normalize_payload(&mut payload, &route.rules)?;
    stamp_omitted_oauth2_flow(&mut payload);
    if !user_is_full_admin(auth) {
        return Err(SessionError::Forbidden(BTreeMap::from([(
            "error".to_owned(),
            FORBIDDEN_MESSAGE.to_owned(),
        )])));
    }
    let created_by = auth
        .request_quota_view()
        .user_id
        .filter(|user_id| !user_id.is_empty())
        .unwrap_or(DEFAULT_CREATED_BY)
        .to_owned();
    let registry = route.registry.read().await;
    let payload = inherit_credentials(&payload, &registry);
    Ok((payload, created_by))
}

// step (g): every failure in this scope becomes the 500 whose text names the caching step
async fn cache_session<Access>(
    route: &SessionRoute<Access>,
    record: &McpServerTable,
    payload: &NewMCPServerRequest,
    server_id: &str,
    created_by: &str,
) -> Result<(), SessionError<Access::Error>>
where
    Access: ServerAccessCheck,
{
    let caching: Result<(), SessionError<Access::Error>> = async {
        let built = route.store.builder.build_server(record, false).await?;
        let cached = route.store.cache_local(built, SESSION_TTL);
        route
            .store
            .persist_draft(payload, server_id, created_by)
            .await?;
        write_redis_copy(&route.store, &cached, SESSION_TTL).await;
        Ok(())
    }
    .await;
    if let Err(error) = caching {
        tracing::error!("Error caching temporary mcp server: {error}");
        return Err(SessionError::Caching {
            source: Box::new(error),
        });
    }
    Ok(())
}

// the facts the wrapper's failure span logs; no span is sent, since no parent span is passed
fn management_failure<Failure>(
    error: SessionError<Failure>,
) -> ManagementFailure<SessionError<Failure>>
where
    Failure: StdError + Send + Sync + 'static,
{
    let exception = ExceptionFacts {
        class_name: "HTTPException".to_owned(),
        class: None,
        message: Some(error.to_string()),
        code: None,
        status_code: Some(error.status().as_u16()),
        response_status_code: None,
        llm_provider: None,
        category: None,
        rate_limit_type: None,
        budget_entity_type: None,
        budget_entity_id: None,
        budget_limit: None,
        budget_spend: None,
        proxy_type: None,
        headers: None,
        response_headers: None,
        litellm_response_headers: None,
        traceback_lines: Vec::new(),
    };
    ManagementFailure { exception, error }
}
