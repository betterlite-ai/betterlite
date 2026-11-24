use std::collections::BTreeSet;
use std::iter::repeat_with;
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
use crate::cost_estimate_route::deployments::DeploymentSet;
use crate::judge_targets::call::JudgeCall;
use crate::judge_targets::error::JudgeDispatchError;
use crate::judge_targets::sdk::JudgeSdk;
use crate::plugin_routes::routes::{Identity, caller_identity};
use crate::shadow_eval_lifecycle::access::require_writer;
use crate::shadow_eval_lifecycle::body::decode_shadow_body;
use crate::shadow_eval_lifecycle::error::LifecycleError;
use crate::shadow_eval_lifecycle::postgres::{PostgresShadowLifecycleStore, store_from_state};
use crate::shadow_eval_lifecycle::start::{StartCall, StartDeps, start_shadow_eval};
use crate::shadow_eval_lifecycle::stop::stop_shadow_eval_job;
use crate::shadow_eval_models::job::ShadowJobResponse;
use crate::shadow_eval_results::{START_PATH, STOP_PATH};
use crate::shadow_eval_validation::request::StartShadowEvalRequest;
use crate::shadow_eval_validation::router_view::{AnthropicCredentials, ShadowEvalRouter};
use crate::state::ServerState;
use crate::tracing_routes::router::unauthenticated;

// the database and the configured-deployment view start-time validation reads, resolved once when the router is built
struct LifecycleDeps {
    store: PostgresShadowLifecycleStore,
    deployments: Option<DeploymentSet>,
}

// what start-time model resolution and validation read, resolved once when the router is built
struct LifecycleState {
    store: Option<Arc<LifecycleDeps>>,
    api_key: Option<SecretString>,
    api_base: Option<String>,
    anthropic: AnthropicSettings,
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
    let lifecycle = Arc::new(lifecycle_state(state));
    Router::new()
        .route(START_PATH, post(start_route))
        .route(STOP_PATH, post(stop_route))
        .with_state(lifecycle)
}

// the route's own state construction, which the test drives so a field mapped to its default fails it
fn lifecycle_state(state: &Arc<ServerState>) -> LifecycleState {
    let credentials = &state.config.sections.anthropic_credentials;
    let sdk = &state.config.sections.resource_sdk_settings;
    LifecycleState {
        store: store_from_state(state).map(|store| {
            Arc::new(LifecycleDeps {
                store,
                // an unindexable model list is logged and leaves no router, so every start answers the unconfigured-400
                deployments: DeploymentSet::from_config(&state.config)
                    .inspect_err(|error| {
                        tracing::warn!(
                            %error,
                            "shadow eval: the configured deployments could not be indexed"
                        );
                    })
                    .ok(),
            })
        }),
        api_key: sdk.api_key.clone(),
        api_base: sdk.api_base.clone(),
        anthropic: AnthropicSettings {
            api_key: credentials.api_key.clone(),
            auth_token: credentials.auth_token.clone(),
            api_base: credentials.api_base.clone(),
            use_bearer_for_custom_base: credentials.use_bearer_for_custom_base,
            anthropic_version: AnthropicSettings::default().anthropic_version,
        },
    }
}

async fn start_route(
    State(state): State<Arc<LifecycleState>>,
    caller: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Response {
    let (store, identity) = match gate(
        &state,
        caller.as_ref(),
        "start a shadow eval",
        StatusCode::CREATED,
    ) {
        Ok(gated) => gated,
        Err(response) => return *response,
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
    match gate(
        &state,
        caller.as_ref(),
        "stop a shadow eval",
        StatusCode::OK,
    ) {
        Ok((store, identity)) => {
            let operator = identity.user_id.as_deref();
            let result = stop_shadow_eval_job(store, &job_id, operator, Utc::now()).await;
            lifecycle_response(result, StatusCode::OK)
        }
        Err(response) => *response,
    }
}

fn gate<'state>(
    state: &'state LifecycleState,
    caller: Option<&Extension<AuthContext>>,
    action: &str,
    success: StatusCode,
) -> Result<(&'state PostgresShadowLifecycleStore, Identity), Box<Response>> {
    let identity = identity_of_caller(caller)?;
    let configured = state.store.as_deref().map(|deps| &deps.store);
    let permitted = require_writer(&identity, action, configured)
        .map_err(|error| Box::new(lifecycle_response(Err(error), success)))?;
    Ok((permitted, identity))
}

fn identity_of_caller(caller: Option<&Extension<AuthContext>>) -> Result<Identity, Box<Response>> {
    let Some(extension) = caller else {
        return Err(Box::new(unauthenticated()));
    };
    caller_identity(&extension.principal).map_err(|error| {
        let detail = LifecycleError::Caller(error).to_string();
        Box::new(
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(DetailBody { detail }),
            )
                .into_response(),
        )
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
    let sdk = UnbuiltJudgeSdk;
    let deps = start_deps(state, &sdk, &resolution, &cache);
    let mut ids = repeat_with(Uuid::new_v4);
    let mut call = StartCall {
        caller,
        now: Utc::now(),
        ids: &mut ids,
    };
    start_shadow_eval(store, deps, request, &mut call).await
}

// the provider inference a wildcard lookup tries after a bare name, which start validation never needs
static INFERRED_PROVIDER: fn(&str) -> Option<String> = |_model| None;

// the router view start validation reads: the configured deployments, or none when the config could not be indexed
fn start_deps<'deps>(
    state: &'deps LifecycleState,
    sdk: &'deps dyn JudgeSdk,
    resolution: &'deps ResolutionContext<'deps>,
    cache: &'deps ProviderCache,
) -> StartDeps<'deps> {
    let router = state
        .store
        .as_deref()
        .and_then(|deps| deps.deployments.as_ref())
        .map(|deployments| ShadowEvalRouter {
            catalog: &deployments.catalog,
            model_group_alias: &deployments.aliases,
            routing_groups: &deployments.groups,
            inferred_provider: &INFERRED_PROVIDER,
        });
    StartDeps {
        router,
        sdk,
        cache,
        resolution,
        credentials: AnthropicCredentials {
            anthropic_key: None,
            api_key: state.api_key.as_ref(),
            settings: &state.anthropic,
            secret_state: SecretManagerReadState {
                client_exists: false,
                settings_exist: false,
                access_mode: SecretAccessMode::ReadOnly,
                hosted_keys: None,
            },
        },
    }
}

mod tests;
