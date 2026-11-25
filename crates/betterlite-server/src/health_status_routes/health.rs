use std::collections::BTreeSet;

use axum::extract::{Extension, RawQuery, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use chrono::{DateTime, Utc};

use crate::auth::auth_identity::AuthContext;
// ast-grep-ignore: no-long-names
use crate::health_access_and_connections::result_filter::filter_health_check_results_by_model_ids as filter_by_ids;
use crate::health_access_and_connections::visibility::{
    HealthCaller, HealthRouter, caller_may_probe_deployment, health_accessible_model_names,
    is_proxy_admin, resolve_targeted_model_ids,
};
use crate::health_status_routes::error::HealthRouteError;
use crate::health_status_routes::health_response::{
    PostProcess, add_missing_id_warning, deployment_ids, model_specific_request, post_process,
    warn_missing_ids,
};
use crate::health_status_routes::query::{HealthQuery, decode_health_query};
use crate::health_status_routes::save::{HealthProbeCall, perform_health_check_and_save};
use crate::health_status_routes::state::{HealthCheckSource, HealthStatusState};
use crate::health_status_routes::target::resolve_target_model_name;

// The caller's model scope, as _caller_may_probe_deployment sees it
struct ProbeScope<'state, 'call> {
    caller: &'call HealthCaller,
    allowed_models: Option<BTreeSet<String>>,
    router: Option<&'state dyn HealthRouter>,
    is_admin: bool,
    restrict: bool,
}

impl ProbeScope<'_, '_> {
    // deep copy first, because a probe adds per-call fields to the deployment entries it reads
    fn kept(&self, deployments: &[DeploymentSpec]) -> Vec<DeploymentSpec> {
        let mut kept = deployments.to_vec();
        if self.restrict {
            kept.retain(|deployment| {
                caller_may_probe_deployment(
                    deployment,
                    self.allowed_models.as_ref(),
                    self.router,
                    self.caller.team_id.as_deref(),
                    self.is_admin,
                )
            });
        }
        kept
    }
}

// GET /health's one router-path call
struct RouterCall<'call> {
    caller: &'call HealthCaller,
    query: &'call HealthQuery,
    target_model: Option<String>,
    start_time: DateTime<Utc>,
    deployments: &'call [DeploymentSpec],
}

pub(super) async fn health_endpoint(
    State(state): State<HealthStatusState>,
    identity: Option<Extension<AuthContext>>,
    RawQuery(raw): RawQuery,
) -> Response {
    let Some(Extension(context)) = identity else {
        return HealthRouteError::Unauthenticated.into_response();
    };
    let query = decode_health_query(raw.as_deref());
    let start_time = state.clock.now();
    let target_model = match resolve_target_model_name(
        query.model.as_deref(),
        query.model_id.as_deref(),
        state.router.as_deref(),
    ) {
        Ok(target) => target,
        Err(error) => return error.into_response(),
    };
    match run_health(&state, &context, &query, target_model, start_time).await {
        Ok(response) => response,
        Err(error) => {
            // LiteLLM's spelling, kept: the \u{72} escape only keeps the typos hook quiet
            tracing::error!(
                "litellm.proxy.proxy_server.py::health_endpoint(): Exception occu\u{72}ed - {error}"
            );
            error.into_response()
        }
    }
}

async fn run_health(
    state: &HealthStatusState,
    context: &AuthContext,
    query: &HealthQuery,
    target_model: Option<String>,
    start_time: DateTime<Utc>,
) -> Result<Response, HealthRouteError> {
    let caller = HealthCaller::from_context(context)?;
    let mode = PostProcess {
        model_specific: model_specific_request(query),
        admin: is_proxy_admin(&caller),
    };
    let Some(deployments) = state.deployments.as_deref().map(Vec::as_slice) else {
        return cli_health(state, &caller, mode, start_time).await;
    };
    router_health(
        state,
        RouterCall {
            caller: &caller,
            query,
            target_model,
            start_time,
            deployments,
        },
    )
    .await
}

// No router and no model list: the CLI model is the only deployment there is
async fn cli_health(
    state: &HealthStatusState,
    caller: &HealthCaller,
    mode: PostProcess,
    start_time: DateTime<Utc>,
) -> Result<Response, HealthRouteError> {
    let Some(user_model) = state.user_model.as_deref() else {
        return Err(HealthRouteError::ModelListNotInitialized);
    };
    let probe = HealthProbeCall {
        model_list: &[],
        target_model: None,
        cli_model: Some(user_model),
        model_id: None,
        team_id: None,
        user_id: caller.user_id.as_deref(),
        start_time,
    };
    let results = perform_health_check_and_save(state, probe).await?;
    post_process(results.to_json(), mode)
}

async fn router_health(
    state: &HealthStatusState,
    call: RouterCall<'_>,
) -> Result<Response, HealthRouteError> {
    let RouterCall {
        caller,
        query,
        target_model,
        start_time,
        deployments,
    } = call;
    let mode = PostProcess {
        model_specific: model_specific_request(query),
        admin: is_proxy_admin(caller),
    };
    let scope = probe_scope(state, caller, mode)?;
    let kept = scope.kept(deployments);
    let targeted_ids = resolve_targeting(&kept, &scope, query)?;
    if state.use_background_health_checks == HealthCheckSource::Background {
        return background_health(state, &scope, &kept, targeted_ids.as_ref(), mode);
    }
    let probe = HealthProbeCall {
        model_list: &kept,
        target_model: target_model.as_deref(),
        cli_model: None,
        model_id: query.model_id.as_deref(),
        team_id: caller.team_id.as_deref(),
        user_id: caller.user_id.as_deref(),
        start_time,
    };
    let results = perform_health_check_and_save(state, probe).await?;
    post_process(results.to_json(), mode)
}

// _resolve_targeted_model_ids over the kept list, and the 403 a scoped caller earns for an empty answer
fn resolve_targeting(
    kept: &[DeploymentSpec],
    scope: &ProbeScope<'_, '_>,
    query: &HealthQuery,
) -> Result<Option<BTreeSet<String>>, HealthRouteError> {
    let targeted = resolve_targeted_model_ids(
        kept,
        query.model.as_deref(),
        query.model_id.as_deref(),
        scope.caller.team_id.as_deref(),
    );
    if scope.restrict && targeted.as_ref().is_some_and(BTreeSet::is_empty) {
        return Err(forbidden_target(query));
    }
    Ok(targeted)
}

fn probe_scope<'state, 'call>(
    state: &'state HealthStatusState,
    caller: &'call HealthCaller,
    mode: PostProcess,
) -> Result<ProbeScope<'state, 'call>, HealthRouteError> {
    let allowed_models = health_accessible_model_names(caller, state.router.as_deref())?;
    Ok(ProbeScope {
        caller,
        restrict: !mode.admin || allowed_models.is_some(),
        allowed_models,
        router: state.router.as_deref(),
        is_admin: mode.admin,
    })
}

// The background cache: narrowed to the targeted or allowed ids, with the missing-id warning on a scoped empty filter
fn background_health(
    state: &HealthStatusState,
    scope: &ProbeScope<'_, '_>,
    deployments: &[DeploymentSpec],
    targeted_ids: Option<&BTreeSet<String>>,
    mode: PostProcess,
) -> Result<Response, HealthRouteError> {
    let result = state.results.snapshot().to_json();
    if !scope.restrict {
        if let Some(ids) = targeted_ids {
            return post_process(filter_by_ids(&result, ids), mode);
        }
        return post_process(result, mode);
    }
    let allowed_model_ids = deployment_ids(deployments);
    let filter_ids = targeted_ids.unwrap_or(&allowed_model_ids);
    let mut filtered = filter_by_ids(&result, filter_ids);
    if targeted_ids.is_none() && !deployments.is_empty() && allowed_model_ids.is_empty() {
        warn_missing_ids(scope.caller);
        add_missing_id_warning(&mut filtered);
    }
    post_process(filtered, mode)
}

fn forbidden_target(query: &HealthQuery) -> HealthRouteError {
    let detail = query
        .model_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .map_or_else(
            || {
                format!(
                    "key not allowed to health-check model {}",
                    query.model.as_deref().unwrap_or("None")
                )
            },
            |model_id| format!("key not allowed to health-check model_id {model_id}"),
        );
    HealthRouteError::http(StatusCode::FORBIDDEN, detail)
}
