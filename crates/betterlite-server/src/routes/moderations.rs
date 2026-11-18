use std::sync::Arc;
use std::time::{Instant, SystemTime};

use async_trait::async_trait;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse as _, Response};
use axum::routing::post;
use axum::{Json, Router};
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
use uuid::Uuid;

use crate::pre_call::rewrites::resolve_call_id;
use crate::routing::simple_shuffle::pick_deployment;
use crate::state::ServerState;

use self::render::{dispatch_failure, missing_deployment, model_required, render_failure};
use super::chat::headers::success_headers;

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
    })
}

#[derive(Clone)]
struct ModerationState {
    calls: Arc<ModerationCalls>,
    dispatch: Arc<Dispatch>,
    moderation_model: Option<String>,
    include_call_id: bool,
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
    let Some(model) = resolve_model(request.model.as_deref(), &state.moderation_model) else {
        return render_failure(&model_required(), &call_id, state.include_call_id);
    };
    let Ok(deployment) = pick_deployment(&state.calls.executor, &model) else {
        return render_failure(&missing_deployment(&model), &call_id, state.include_call_id);
    };
    execute(&state, request, deployment, &call_id, started).await
}

// resolve_inference_model's moderation arm: the configured default overrides the body, and the launch model is unset
fn resolve_model(body_model: Option<&str>, moderation_model: &Option<String>) -> Option<String> {
    let defaults = ModelDefaults {
        completion_model: None,
        image_generation_model: None,
        moderation_model: moderation_model.clone(),
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
    let operation = Operation::Moderation(DispatchRequest {
        call_id: call_id.to_owned(),
        model_id: deployment.model_id.clone(),
        request: Box::new(request),
        dispatch: Arc::clone(&state.dispatch),
        calls: Arc::clone(&state.calls),
    });
    let context = DispatchContext::start(CallInputs {
        now: SystemTime::now,
        id: Uuid::new_v4,
        deadline: None,
        privacy: false,
    });
    let redactor = &state.calls.executor.redactor;
    match state.dispatch.execute(operation, &context).await {
        Ok(CallResult::Moderation(result)) => {
            // proxy_server.py's moderation handler gives get_custom_headers no provider header, so none is forwarded
            let headers = success_headers(
                &HeaderMap::new(),
                deployment,
                call_id,
                result.upstream_time,
                started.elapsed(),
            );
            (headers, Json(result.response)).into_response()
        }
        Ok(_) => dispatch_failure(
            DispatchError::NotBuilt {
                entry: "moderation",
            },
            call_id,
            state.include_call_id,
            redactor,
        ),
        Err(error) => dispatch_failure(error, call_id, state.include_call_id, redactor),
    }
}
