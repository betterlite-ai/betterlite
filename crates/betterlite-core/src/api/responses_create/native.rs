use std::time::Duration;

use betterlite_config::env_integer::timeout::RequestTimeout;
use betterlite_context::non_default_params::Params;
use betterlite_models::call_helpers::drop_params::normalize_drop_params;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::{HiddenParams, Json};
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::response_input::{ResponsesInput, ResponsesItem};
use secrecy::ExposeSecret as _;
use serde_json::Number;

use crate::api::response_id_traversal::metadata::AffinityMetadata;
use crate::api::response_id_traversal::record::Record;
use crate::api::response_id_traversal::response_id::rewrite_typed_response;
use crate::api::response_id_traversal::restore::restore_encrypted_item_ids;
use crate::api::responses_agentic_execution::executor::ContinuationExecutor;
use crate::api::responses_compact_http::service::{
    ClientKind, ClientSource, LogContext, SuppliedClient,
};
use crate::api::responses_create_http::config::{CreateParams, Params as HttpParams};
use crate::api::responses_create_http::service::{CreateCall, CreateDeps, CreateOutcome, run};
use crate::api::responses_hook_policy::callbacks::DeploymentHookDispatcher;
use crate::api::responses_parameters::optional::optional_params_responses;
use crate::api::responses_parameters::provider::DropPolicy;
use crate::api::responses_provider_selection::ParameterMap;
use crate::api::responses_provider_selection::bridging::bridge_kwargs;
use crate::api::responses_provider_selection::provider_config::ResponsesApiConfig;

use super::bridge::{BridgeCall, Created, StampedResponse};
use super::call::ResponsesCreateCall;
use super::convert::{
    decision_json, decision_prompt, object_from_params, object_value, prompt_decision, reencode,
};
use super::deps::{CreateConfigFactory, ResponsesCreateDeps};
use super::error::CreateError;
use super::logging;
use super::preparation::{Prepared, Resolved};

// The native tail: filter the options through the provider, log, restore IDs, POST, then stamp the response
pub(super) async fn native<S, F, D, E, Context>(
    deps: &ResponsesCreateDeps<'_, S, F, D, E, Context>,
    prep: &Prepared<'_>,
    call: &ResponsesCreateCall,
    resolved: &Resolved,
    provider_config: &F::Provider,
    mut requested: Params,
    request_timeout: RequestTimeout,
    log: &mut LogContext<'_, '_>,
) -> Result<Created<Context>, CreateError>
where
    S: ClientSource,
    F: CreateConfigFactory,
    D: DeploymentHookDispatcher + ?Sized,
    E: ContinuationExecutor + ?Sized,
    Context: Send,
{
    let allowed = call.allowed_openai_params.clone().unwrap_or_default();
    let filtered = optional_params_responses(
        provider_config,
        &resolved.model,
        &mut requested,
        &allowed,
        drop_policy(&prep.kwargs, deps.drop_params),
    )
    .map_err(CreateError::from)?;
    logging::log_call(
        &mut *log.state,
        call,
        resolved,
        &filtered,
        prep.is_async,
        &prep.kwargs,
    )?;
    let input = restored(&prep.input);
    let optional_params = object_from_params(&filtered);
    let params = create_params(resolved, call, &prep.kwargs)?;
    let metadata = litellm_metadata(&prep.kwargs)?;
    let timeout = native_timeout(call.timeout, request_timeout)?;
    let fake = Json::Bool(provider_config.should_fake_stream(
        &resolved.model,
        call.stream,
        &resolved.provider,
    ));
    let mode = if prep.is_async {
        ClientKind::Asynchronous
    } else {
        ClientKind::Sync
    };
    let http = CreateDeps {
        config: provider_config,
        source: deps.source,
        supplied: deps.supplied.as_ref().map(|supplied| SuppliedClient {
            kind: supplied.kind,
            transport: supplied.transport,
        }),
    };
    let create_call = CreateCall {
        model: &resolved.model,
        input: &input,
        optional_params: &optional_params,
        custom_llm_provider: &resolved.provider,
        params: &params,
        extra_headers: prep.extra_headers.as_ref(),
        extra_body: call.extra_body.as_ref(),
        timeout: Some(timeout),
        fake_stream: Some(&fake),
        litellm_metadata: metadata.as_ref(),
        shared_session: call.shared_session,
    };
    let outcome = run(&http, mode, &create_call, &deps.hooks, log).await?;
    finish(outcome, resolved, metadata.as_ref())
}

// The bridged tail: litellm_completion_transformation_handler.response_api_handler, then its answer as the call's result
pub(super) async fn bridge<S, F, D, E, Context>(
    deps: &ResponsesCreateDeps<'_, S, F, D, E, Context>,
    prep: &Prepared<'_>,
    call: &ResponsesCreateCall,
    config: Option<&ResponsesApiConfig>,
    requested: &Params,
    request_timeout: RequestTimeout,
    context: Context,
) -> Result<Created<Context>, CreateError>
where
    S: ClientSource,
    F: CreateConfigFactory,
    D: DeploymentHookDispatcher + ?Sized,
    E: ContinuationExecutor + ?Sized,
    Context: Send,
{
    let allowed = call.allowed_openai_params.clone().unwrap_or_default();
    let mut kwargs = bridge_kwargs(&prep.kwargs, config, &allowed);
    let seconds = call.timeout.unwrap_or(request_timeout.seconds);
    let Some(timeout) = Number::from_f64(seconds) else {
        return Err(CreateError::InvalidTimeout { seconds });
    };
    kwargs.insert(String::from("timeout"), JsonValue::Number(timeout));
    kwargs.insert(
        String::from("extra_body"),
        call.extra_body
            .as_ref()
            .map_or(JsonValue::Null, object_value),
    );
    kwargs.insert(
        String::from("allowed_openai_params"),
        call.allowed_openai_params
            .as_ref()
            .map_or(JsonValue::Null, |names| {
                JsonValue::Array(names.iter().cloned().map(JsonValue::Text).collect())
            }),
    );
    let request = object_from_params(requested);
    let bridge_call = BridgeCall {
        model: &prep.model,
        input: &prep.input,
        request: &request,
        custom_llm_provider: prep.provider.as_deref(),
        is_async: prep.is_async,
        stream: call.stream,
        extra_headers: prep.extra_headers.as_ref(),
        kwargs: &kwargs,
    };
    Ok(deps.bridge.respond(&bridge_call, context).await?)
}

// _update_responses_api_response_id_with_model_id, then the provider stamp the callbacks read
fn finish<Context>(
    outcome: CreateOutcome,
    resolved: &Resolved,
    metadata: Option<&HttpParams>,
) -> Result<Created<Context>, CreateError> {
    match outcome {
        CreateOutcome::Value(raw) => {
            match serde_json::from_str::<ResponsesApiResponse>(raw.get()) {
                Ok(mut response) => {
                    let affinity: AffinityMetadata = match metadata {
                        Some(map) => reencode(map)?,
                        None => AffinityMetadata::default(),
                    };
                    rewrite_typed_response(&mut response, Some(&resolved.provider), &affinity)?;
                    let mut hidden = HiddenParams::new();
                    hidden.insert(
                        String::from("custom_llm_provider"),
                        Json::Text(resolved.provider.clone()),
                    );
                    Ok(Created::Responses(Box::new(StampedResponse {
                        response,
                        hidden,
                    })))
                }
                Err(_) => Ok(Created::Raw(raw)),
            }
        }
        CreateOutcome::Live(live) => Ok(Created::Live(live)),
        CreateOutcome::Synthetic(synthetic) => Ok(Created::Synthetic(synthetic)),
    }
}

// _restore_encrypted_content_item_ids_in_input decodes the litellm-encoded item IDs before the request is sent
fn restored(input: &ResponsesInput) -> ResponsesInput {
    let ResponsesInput::Items(items) = input else {
        return input.clone();
    };
    let mut record = Record::List(
        items
            .iter()
            .map(|item| Record::from_json(decision_prompt(&item.0)))
            .collect(),
    );
    restore_encrypted_item_ids(&mut record);
    let Json::List(entries) = record.into_json() else {
        return input.clone();
    };
    ResponsesInput::Items(
        entries
            .into_iter()
            .map(|item| ResponsesItem(prompt_decision(&item)))
            .collect(),
    )
}

// dict(litellm_params) with the base, key and TLS choice the settings resolved
fn create_params(
    resolved: &Resolved,
    call: &ResponsesCreateCall,
    kwargs: &ParameterMap,
) -> Result<CreateParams, CreateError> {
    let mut map: HttpParams = reencode(kwargs)?;
    if let Some(base) = &resolved.api_base {
        map.insert(String::from("api_base"), Json::Text(base.clone()));
    }
    if let Some(key) = &resolved.api_key {
        map.insert(
            String::from("api_key"),
            Json::Text(key.expose_secret().to_owned()),
        );
    }
    if let Some(verify) = call.ssl_verify {
        map.insert(String::from("ssl_verify"), Json::Bool(verify));
    }
    CreateParams::from_map(map).map_err(CreateError::from)
}

// kwargs.get("litellm_metadata", {}): absent is an empty object, an explicit null stays empty too
fn litellm_metadata(kwargs: &ParameterMap) -> Result<Option<HttpParams>, CreateError> {
    match kwargs.get("litellm_metadata") {
        None => Ok(Some(HttpParams::new())),
        Some(JsonValue::Null) => Ok(None),
        Some(value @ JsonValue::Object(_)) => reencode(value).map(Some).map_err(CreateError::from),
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Text(_) | JsonValue::Array(_),
        ) => Err(CreateError::Field {
            field: "litellm_metadata",
            expected: "an object",
        }),
    }
}

fn drop_policy(kwargs: &ParameterMap, global: bool) -> DropPolicy {
    let value = kwargs.get("drop_params").map(decision_json);
    DropPolicy {
        global,
        call: normalize_drop_params(value.as_ref()),
    }
}

// `timeout or request_timeout`: zero falls back, and the fallback reads REQUEST_TIMEOUT with its 6000s default
fn native_timeout(
    configured: Option<f64>,
    request_timeout: RequestTimeout,
) -> Result<Duration, CreateError> {
    let seconds = configured
        .filter(|value| *value != 0.0_f64)
        .unwrap_or(request_timeout.seconds);
    Duration::try_from_secs_f64(seconds).map_err(|_error| CreateError::InvalidTimeout { seconds })
}
