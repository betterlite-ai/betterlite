use std::backtrace::Backtrace;
use std::sync::Arc;
use std::time::SystemTime;

use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::routes::{
    ApplyGuardrailRequest, ApplyGuardrailResponse,
};
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::inputs::GuardrailApiInputs;
use betterlite_guardrails::guardrail_records::error::GuardrailRecordError;
use betterlite_guardrails::guardrail_records::invocation::GuardrailHookName;
use betterlite_guardrails::guardrail_records::invocation::GuardrailInvocation;
use betterlite_guardrails::guardrail_records::record::GuardrailRecordContext;
use betterlite_guardrails::guardrail_registry::callback::GuardrailCallback;

use crate::auth::auth_identity::AuthContext;
use crate::guardrails::callback_caps::errors::ProxyHookError;
use crate::registry_read_through::Resync;
// ast-grep-ignore: no-long-names
use crate::registry_read_through::lookup::{GuardrailLookup, get_guardrail_with_read_through};
use crate::tracing_routes::router::unauthenticated;

use super::CatalogApplyState;
use super::apply_data::{call_data, sent_request_data};
use super::body::decode_body;
use super::error::CatalogApplyError;
use super::input_type::resolve;
use super::ports::{APPLY_CALL_TYPE, ApplyHooks, ApplyLog, ApplyPorts, Body};
use super::success_logs::{self, SuccessLogging};

const APPLY_PATH: &str = "/guardrails/apply_guardrail";
const ALIAS_PATH: &str = "/apply_guardrail";

// the pre-call data and the logging handle stay in scope so the failure path can reach them
struct Exchange {
    data: Body,
    log: Option<Arc<dyn ApplyLog>>,
}

pub(super) fn routes<L, R>(state: CatalogApplyState<L, R>) -> Router
where
    L: GuardrailLookup + 'static,
    R: Resync + 'static,
{
    Router::new()
        .route(APPLY_PATH, post(apply_guardrail::<L, R>))
        .route(ALIAS_PATH, post(apply_guardrail::<L, R>))
        .with_state(state)
}

async fn apply_guardrail<L, R>(
    State(state): State<CatalogApplyState<L, R>>,
    caller: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Response
where
    L: GuardrailLookup + 'static,
    R: Resync + 'static,
{
    let Some(Extension(caller)) = caller else {
        return unauthenticated();
    };
    let request: ApplyGuardrailRequest = match decode_body(&body) {
        Ok(request) => request,
        Err(error) => return error.into_response(),
    };
    let started_at = SystemTime::now();
    let mut exchange = Exchange {
        data: call_data(&request),
        log: None,
    };
    let hooks = state.ports.hooks.as_ref();
    match attempt(&state, &caller, &request, &mut exchange).await {
        Ok(response) => answer(hooks, &caller, started_at, response, &exchange).await,
        Err(error) => failure(hooks, &caller, error, &exchange).await,
    }
}

async fn answer(
    hooks: &dyn ApplyHooks,
    caller: &AuthContext,
    started_at: SystemTime,
    response: ApplyGuardrailResponse,
    exchange: &Exchange,
) -> Response {
    let logging = SuccessLogging {
        hooks,
        caller,
        data: &exchange.data,
        log: exchange.log.as_deref(),
        started_at,
    };
    let response = success_logs::emit(response, &logging).await;
    (StatusCode::OK, Json(response)).into_response()
}

async fn attempt<L, R>(
    state: &CatalogApplyState<L, R>,
    caller: &AuthContext,
    request: &ApplyGuardrailRequest,
    exchange: &mut Exchange,
) -> Result<ApplyGuardrailResponse, CatalogApplyError>
where
    L: GuardrailLookup + 'static,
    R: Resync + 'static,
{
    let callback = lookup(state, &request.guardrail_name).await?;
    let hooks = state.ports.hooks.as_ref();
    let outcome = hooks
        .pre_call(exchange.data.clone(), caller, APPLY_CALL_TYPE)
        .await?;
    exchange.data = outcome.data;
    exchange.log = outcome.log;
    if let Some(log) = exchange.log.as_deref() {
        log.mark_pass_through(success_logs::pass_through_messages(request))?;
    }
    let mut sent = sent_request_data(request);
    let guardrail_hook = callback.base.settings.event_hook.as_ref();
    let input_type = resolve(guardrail_hook, &request.input_type);
    let ports = state.ports.as_ref();
    let returned = invoke(
        ports,
        callback,
        GuardrailApiInputs {
            texts: Some(vec![request.text.clone()]),
            images: None,
            tools: None,
            tool_calls: None,
            structured_messages: None,
            model: None,
            stream_holdback_chars: None,
        },
        &mut sent,
        input_type,
    )
    .await?;
    Ok(ApplyGuardrailResponse {
        response_text: first_text(&returned, &request.text),
    })
}

async fn lookup<'state, L, R>(
    state: &'state CatalogApplyState<L, R>,
    guardrail_name: &str,
) -> Result<&'state GuardrailCallback, CatalogApplyError>
where
    L: GuardrailLookup,
    R: Resync,
{
    // ast-grep-ignore: no-long-names
    let found = get_guardrail_with_read_through(
        state.lookup.as_ref(),
        state.read_through.as_ref(),
        guardrail_name,
    )
    .await;
    match found {
        Some(callback) => Ok(callback),
        None => Err(CatalogApplyError::GuardrailNotFound {
            guardrail_name: guardrail_name.to_owned(),
        }),
    }
}

async fn invoke(
    ports: &ApplyPorts,
    callback: &GuardrailCallback,
    inputs: GuardrailApiInputs,
    request_data: &mut Body,
    input_type: GuardrailInputType,
) -> Result<GuardrailApiInputs, CatalogApplyError> {
    let invocation = GuardrailInvocation::begin(
        GuardrailHookName::ApplyGuardrail,
        request_data,
        Some(&inputs),
        SystemTime::now(),
    )?;
    let outcome = ports
        .runner
        .apply_guardrail(callback, inputs, request_data, input_type)
        .await;
    let mut context = GuardrailRecordContext {
        base: &callback.base,
        request_data,
        log_target: None,
        sink: ports.sink.as_ref(),
    };
    let failure = match outcome {
        Ok(returned) => {
            let summary = serde_json::to_value(&returned)
                .and_then(serde_json::from_value::<GuardrailValue>)
                .map_err(|source| {
                    CatalogApplyError::Record(GuardrailRecordError::Json { source })
                })?;
            invocation.finish_success(&mut context, Some(&summary), SystemTime::now())?;
            return Ok(returned);
        }
        Err(failure) => invocation.finish_failure(&mut context, failure, SystemTime::now())?,
    };
    Err(CatalogApplyError::Guardrail(failure))
}

async fn failure(
    hooks: &dyn ApplyHooks,
    caller: &AuthContext,
    error: CatalogApplyError,
    exchange: &Exchange,
) -> Response {
    let error = failure_handlers(exchange.log.as_deref(), error).await;
    let error = hooked_failure(hooks, caller, error, &exchange.data).await;
    error.into_response()
}

async fn failure_handlers(
    log: Option<&dyn ApplyLog>,
    error: CatalogApplyError,
) -> CatalogApplyError {
    let Some(log) = log else {
        return error;
    };
    if error.is_http() {
        return error;
    }
    let traceback = format!("{:?}", Backtrace::capture());
    if log.async_failure_handler(&error, &traceback).await.is_err() {
        tracing::error!("apply_guardrail: async_failure_handler failed");
    }
    if log.submit_failure_handler(&error, &traceback).is_err() {
        tracing::error!("apply_guardrail: failure_handler submit failed");
    }
    error
}

// the failure hook may replace the error, and a failing hook itself leaves the error alone
async fn hooked_failure(
    hooks: &dyn ApplyHooks,
    caller: &AuthContext,
    error: CatalogApplyError,
    data: &Body,
) -> CatalogApplyError {
    match hooks.post_call_failure(caller, &error, data).await {
        Ok(Some(replacement)) => CatalogApplyError::Hooks(ProxyHookError::from(replacement)),
        Ok(None) => error,
        Err(hook_error) => {
            tracing::error!(
                error = %hook_error,
                "apply_guardrail: post_call_failure_hook failed"
            );
            error
        }
    }
}

fn first_text(returned: &GuardrailApiInputs, request_text: &str) -> String {
    returned
        .texts
        .as_ref()
        .and_then(|texts| texts.first())
        .cloned()
        .unwrap_or_else(|| request_text.to_owned())
}
