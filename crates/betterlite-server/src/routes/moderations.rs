use std::sync::Arc;
use std::time::{Instant, SystemTime};

use async_trait::async_trait;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse as _, Response};
use axum::routing::post;
use axum::{Json, Router};
use betterlite_config::sections::detailed_timing::DetailedTiming;
use betterlite_config::sections::settings::JsonValue;
use betterlite_context::call_context::{CallContext as DispatchContext, CallInputs};
use betterlite_core::api::request_destination::model::{
    InferenceModelKind, ModelDefaults, resolve_inference_model,
};
use betterlite_core::dispatch::error::DispatchError;
use betterlite_core::dispatch::log::TracingCallLog;
use betterlite_core::dispatch::{Dispatch, NoRules};
use betterlite_core::operation::CallResult;
use betterlite_core::operation::Operation;
use betterlite_core::operation::moderation::{
    ModerationCall, ModerationCallInput, ModerationFailure, ModerationRequest as DispatchRequest,
    ModerationResult, route_error,
};
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::ChatExecutor;
use betterlite_openai::moderation_execution::{ModerationError, send};
use betterlite_openai::moderation_request::ModerationRequest;
use betterlite_openai::openai_base_config::Object as JsonMap;
use uuid::Uuid;

use crate::custom_headers::custom_headers;
use crate::custom_headers::input::{CustomHeadersInput, KeyHeaderView};
use crate::pre_call::rewrites::resolve_call_id;
use crate::routing::simple_shuffle::pick_deployment;
use crate::state::ServerState;

use self::render::{dispatch_failure, refusal, render_failure};

mod render;
mod tests;

pub(crate) fn routes(state: &Arc<ServerState>) -> Router {
    let executor = &state.executor;
    let dispatch = Arc::new(Dispatch::new(
        executor.deployments.clone(),
        Arc::new(NoRules),
        Arc::new(TracingCallLog),
    ));
    let mut router = Router::new();
    router = router.route("/moderations", post(moderations));
    router = router.route("/v1/moderations", post(moderations));
    router.with_state(ModerationState {
        calls: Arc::new(ModerationCalls {
            executor: Arc::clone(executor),
        }),
        dispatch,
        moderation_model: state
            .config
            .sections
            .moderation_model
            .moderation_model
            .clone(),
        include_call_id: state
            .config
            .sections
            .error_body_call_id
            .include_call_id_in_error_body,
        detailed_timing: state.config.sections.detailed_timing,
    })
}

#[derive(Clone)]
struct ModerationState {
    calls: Arc<ModerationCalls>,
    dispatch: Arc<Dispatch>,
    moderation_model: Option<String>,
    include_call_id: bool,
    detailed_timing: DetailedTiming,
}

// the OpenAI moderation call sits behind the dispatch port, so the route keeps no provider code
struct ModerationCalls {
    executor: Arc<ChatExecutor>,
}

#[async_trait]
impl ModerationCall for ModerationCalls {
    async fn moderate(
        &self,
        call: ModerationCallInput<'_>,
    ) -> Result<ModerationResult, ModerationFailure> {
        send(&self.executor, &call.deployment, call.request)
            .await
            .map(|outcome| ModerationResult {
                response: outcome.response,
                upstream_headers: outcome.upstream.headers,
                upstream_time: outcome.upstream_time,
            })
            .map_err(|error| route_error(&error, &self.executor.redactor))
    }
}

async fn moderations(
    State(state): State<ModerationState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let started = Instant::now();
    let call_id = resolve_call_id(
        headers
            .get("x-litellm-call-id")
            .and_then(|value| value.to_str().ok()),
    );
    let request = match ModerationRequest::parse(&body) {
        Ok(request) => request,
        Err(error) => {
            let refused = route_error(
                &ModerationError::Request(error),
                &state.calls.executor.redactor,
            );
            return render_failure(&refused, &call_id, state.include_call_id);
        }
    };
    let Some(model) = resolve_model(request.model.as_deref(), state.moderation_model.as_deref())
    else {
        return render_failure(&refusal(None), &call_id, state.include_call_id);
    };
    let Ok(deployment) = pick_deployment(&state.calls.executor, &model) else {
        return render_failure(&refusal(Some(&model)), &call_id, state.include_call_id);
    };
    execute(&state, request, deployment, &call_id, started).await
}

// resolve_inference_model's moderation arm: the configured default overrides the body, and the launch model is unset
fn resolve_model(body_model: Option<&str>, moderation_model: Option<&str>) -> Option<String> {
    let defaults = ModelDefaults {
        completion_model: None,
        image_generation_model: None,
        moderation_model: moderation_model.map(String::from),
    };
    resolve_inference_model(
        body_model,
        &defaults,
        None,
        None,
        InferenceModelKind::Moderation,
    )
}

async fn execute(
    state: &ModerationState,
    request: ModerationRequest,
    deployment: &ChatDeployment,
    call_id: &str,
    started: Instant,
) -> Response {
    let held: Arc<ModerationCalls> = Arc::clone(&state.calls);
    let calls: Arc<dyn ModerationCall> = held;
    let operation = Operation::Moderation(DispatchRequest {
        call_id: call_id.to_owned(),
        model_id: deployment.model_id.clone(),
        request: Box::new(request),
        dispatch: Arc::clone(&state.dispatch),
        calls,
    });
    let context = DispatchContext::start(CallInputs {
        now: SystemTime::now,
        id: Uuid::new_v4,
        deadline: None,
        privacy: false,
    });
    let redactor = &state.calls.executor.redactor;
    match state.dispatch.execute(operation, &context).await {
        Ok(CallResult::Moderation(result)) => answered(state, deployment, result, call_id, started),
        Ok(_) => render_failure(
            &dispatch_failure(
                DispatchError::NotBuilt {
                    entry: "moderation",
                },
                redactor,
            ),
            call_id,
            state.include_call_id,
        ),
        Err(error) => render_failure(
            &dispatch_failure(error, redactor),
            call_id,
            state.include_call_id,
        ),
    }
}

// proxy_server.py's moderation handler gives get_custom_headers the served model id, api base and the response's hidden params, but no cache key or logging object
fn answered(
    state: &ModerationState,
    deployment: &ChatDeployment,
    result: ModerationResult,
    call_id: &str,
    started: Instant,
) -> Response {
    let mut hidden_params = JsonMap::new();
    let response_ms = started.elapsed().as_secs_f64().mul_add(1000.0_f64, 0.0_f64);
    hidden_params.insert(String::from("_response_ms"), JsonValue::from(response_ms));
    // update_response_metadata writes the logging object's callback_duration_ms, 0.0 because betterlite runs no callbacks
    hidden_params.insert(
        String::from("callback_duration_ms"),
        JsonValue::from(0.0_f64),
    );
    let input = CustomHeadersInput {
        key: KeyHeaderView::default(),
        call_id: Some(call_id.to_owned()),
        model_id: Some(deployment.model_id.clone()),
        cache_key: None,
        api_base: Some(deployment.api_base.clone()),
        version: Some(env!("CARGO_PKG_VERSION").to_owned()),
        model_region: None,
        response_cost: None,
        hidden_params,
        fastest_batch: None,
        request_data: JsonMap::new(),
        timeout: None,
        logging: None,
        read_timing_from_logging: true,
        extra: Vec::new(),
    };
    let headers = custom_headers(&input, state.detailed_timing);
    (headers, Json(result.response)).into_response()
}
