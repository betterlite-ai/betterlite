use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use axum::routing::post;
use betterlite_anthropic::credentials::AnthropicSettings;
use betterlite_config::loader::startup::process_environment;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::decisions::Json as JsonValue;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::judge_primitives::provider_qualified::ProviderCache;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_secret_managers::secret_helpers::SecretManagerReadState;
use betterlite_secret_managers::secret_types::SecretAccessMode;
use chrono::Utc;
use secrecy::SecretString;
use serde::Serialize;
use uuid::Uuid;

use crate::auth::auth_identity::AuthContext;
use crate::judge_targets::call::JudgeCall;
use crate::judge_targets::error::JudgeDispatchError;
use crate::judge_targets::sdk::JudgeSdk;
use crate::plugin_routes::routes::{Identity, caller_identity};
use crate::shadow_eval_lifecycle::access::require_writer;
use crate::shadow_eval_lifecycle::body::decode_shadow_body;
use crate::shadow_eval_lifecycle::error::LifecycleError;
use crate::shadow_eval_lifecycle::postgres::{PostgresShadowLifecycleStore, store_from_state};
use crate::shadow_eval_lifecycle::start::{StartDeps, start_shadow_eval};
use crate::shadow_eval_lifecycle::stop::stop_shadow_eval_job;
use crate::shadow_eval_models::job::ShadowJobResponse;
use crate::shadow_eval_validation::request::StartShadowEvalRequest;
use crate::shadow_eval_validation::router_view::AnthropicCredentials;
use crate::state::ServerState;
use crate::tracing_routes::router::unauthenticated;

// the two globals start-time model resolution reads, resolved once when the router is built
struct LifecycleState {
    store: Option<Arc<PostgresShadowLifecycleStore>>,
    api_key: Option<SecretString>,
    api_base: Option<String>,
}

#[derive(Serialize)]
struct DetailBody {
    detail: String,
}

// start-time validation resolves models without dispatching a judge call, so the judge sdk answers only there
struct UnbuiltJudgeSdk;

#[async_trait]
impl JudgeSdk for UnbuiltJudgeSdk {
    async fn acompletion(&self, _call: JudgeCall) -> Result<JsonValue, JudgeDispatchError> {
        Err(JudgeDispatchError {
            message: "start-time validation never dispatches a judge call".to_owned(),
        })
    }
}

#[cfg_attr(test, mutants::skip)]
pub(crate) fn routes(state: &Arc<ServerState>) -> Router {
    let lifecycle = Arc::new(LifecycleState {
        store: store_from_state(state).map(Arc::new),
        api_key: state.config.sections.resource_sdk_settings.api_key.clone(),
        api_base: state.config.sections.resource_sdk_settings.api_base.clone(),
    });
    Router::new()
        .route("/auto_router/shadow_eval/start", post(start_route))
        .route("/auto_router/shadow_eval/{job_id}/stop", post(stop_route))
        .with_state(lifecycle)
}

async fn start_route(
    State(state): State<Arc<LifecycleState>>,
    caller: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Response {
    let identity = match identity_of_caller(caller.as_ref()) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let store = match require_writer(&identity, "start a shadow eval", state.store.as_deref()) {
        Ok(store) => store,
        Err(error) => return lifecycle_response(Err(error), StatusCode::CREATED),
    };
    let request = match decode_shadow_body(&body) {
        Ok(request) => request,
        Err(error) => {
            return lifecycle_response(Err(LifecycleError::from(error)), StatusCode::CREATED);
        }
    };
    let result = start_job(&state, store, &request, identity.user_id.as_deref()).await;
    lifecycle_response(result, StatusCode::CREATED)
}

async fn stop_route(
    State(state): State<Arc<LifecycleState>>,
    Path(job_id): Path<String>,
    caller: Option<Extension<AuthContext>>,
) -> Response {
    let identity = match identity_of_caller(caller.as_ref()) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let store = match require_writer(&identity, "stop a shadow eval", state.store.as_deref()) {
        Ok(store) => store,
        Err(error) => return lifecycle_response(Err(error), StatusCode::OK),
    };
    let result =
        stop_shadow_eval_job(store, &job_id, identity.user_id.as_deref(), Utc::now()).await;
    lifecycle_response(result, StatusCode::OK)
}

fn identity_of_caller(caller: Option<&Extension<AuthContext>>) -> Result<Identity, Response> {
    let Some(caller) = caller else {
        return Err(unauthenticated());
    };
    caller_identity(&caller.principal).map_err(|error| {
        let detail = LifecycleError::Caller(error).to_string();
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(DetailBody { detail }),
        )
            .into_response()
    })
}

fn lifecycle_response(
    result: Result<ShadowJobResponse, LifecycleError>,
    success: StatusCode,
) -> Response {
    let error = match result {
        Ok(job) => return (success, Json(job)).into_response(),
        Err(error) => error,
    };
    let status = match error.status_code() {
        400 => StatusCode::BAD_REQUEST,
        401 => StatusCode::UNAUTHORIZED,
        403 => StatusCode::FORBIDDEN,
        404 => StatusCode::NOT_FOUND,
        409 => StatusCode::CONFLICT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        status,
        Json(DetailBody {
            detail: error.to_string(),
        }),
    )
        .into_response()
}

async fn start_job(
    state: &LifecycleState,
    store: &PostgresShadowLifecycleStore,
    request: &StartShadowEvalRequest,
    caller: Option<&str>,
) -> Result<ShadowJobResponse, LifecycleError> {
    let mut bundled = BundledCatalog::default();
    let catalog = bundled.get();
    let no_names = BTreeSet::new();
    let environment = process_environment();
    let resolution = ResolutionContext {
        catalog: catalog.as_ref(),
        known_model_names: &no_names,
        custom_providers: &no_names,
        environment: &environment,
        registry: JSONProviderRegistry::load(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials {
            api_key: state.api_key.clone(),
            api_base: state.api_base.clone(),
            xai_key: None,
            inception_key: None,
            lemonade_key: None,
        },
    };
    let cache = ProviderCache::default();
    let settings = AnthropicSettings::default();
    let deps = StartDeps {
        // this server installs no live DeploymentCatalog yet, so a start answers the no-router 400 until it does
        router: None,
        sdk: &UnbuiltJudgeSdk,
        cache: &cache,
        resolution: &resolution,
        credentials: AnthropicCredentials {
            anthropic_key: None,
            api_key: state.api_key.as_ref(),
            settings: &settings,
            secret_state: SecretManagerReadState {
                client_exists: false,
                settings_exist: false,
                access_mode: SecretAccessMode::ReadOnly,
                hosted_keys: None,
            },
        },
    };
    let mut ids = std::iter::repeat_with(Uuid::new_v4);
    start_shadow_eval(store, deps, request, caller, Utc::now(), &mut ids).await
}
