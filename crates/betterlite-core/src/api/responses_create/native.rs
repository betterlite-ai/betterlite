use super::bridge::{BridgeCall, Created, StampedResponse};
use super::deps::{CreateCapabilities, CreateConfigFactory, ResponsesCreateDeps};
use super::error::CreateError;
use super::logging;
use super::preparation::{Prepared, Resolved};
use crate::api::response_id_traversal::metadata::AffinityMetadata;
use crate::api::response_id_traversal::response_id::{rewrite_typed_response, transcode};
use crate::api::response_id_traversal::restore::restore_input_item_ids;
use crate::api::responses_agentic_execution::executor::ContinuationExecutor;
use crate::api::responses_compact_http::service::{ClientKind, ClientSource, SuppliedClient};
use crate::api::responses_create_http::config::{CreateParams, Params as HttpParams};
use crate::api::responses_create_http::service::{
    CreateCall, CreateDeps, CreateOutcome, CreateTransport, LogContext, run,
};
use crate::api::responses_hook_policy::callbacks::DeploymentHookDispatcher;
use crate::api::responses_json::{decision_json, object_from_params, object_value};
use crate::api::responses_parameters::optional::optional_params_responses;
use crate::api::responses_parameters::provider::DropPolicy;
use crate::api::responses_provider_selection::ParameterMap;
use crate::api::responses_provider_selection::bridging::bridge_kwargs;
use crate::api::responses_provider_selection::provider_config::ResponsesApiConfig;
use betterlite_config::env_integer::timeout::RequestTimeout;
use betterlite_context::non_default_params::Params;
use betterlite_models::call_helpers::drop_params::normalize_drop_params;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::{HiddenParams, Json};
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::response_input::ResponsesInput;
use betterlite_prompts::arguments::json::Object;
use secrecy::ExposeSecret as _;
use serde_json::Number;
pub(super) struct NativeCall<'call, Provider> {
    pub(super) prep: &'call Prepared<'call>,
    pub(super) resolved: &'call Resolved,
    pub(super) provider_config: &'call Provider,
    pub(super) requested: Params,
    pub(super) timeout: RequestTimeout,
}
struct RequestParts {
    input: ResponsesInput,
    optional_params: Object,
    params: CreateParams,
    metadata: Option<HttpParams>,
    seconds: f64,
    fake: Json,
}
pub(super) async fn native<S, F, D, E, Context>(
    deps: &ResponsesCreateDeps<'_, S, F, D, E, Context>,
    call: NativeCall<'_, F::Provider>,
    log: &mut LogContext<'_, '_>,
) -> Result<Created<Context>, CreateError>
where
    S: ClientSource<Client: CreateTransport>,
    F: CreateConfigFactory,
    D: DeploymentHookDispatcher + ?Sized,
    E: ContinuationExecutor + ?Sized,
    Context: Send,
{
    let NativeCall {
        prep,
        resolved,
        provider_config,
        mut requested,
        timeout,
    } = call;
    let filtered = filtered_params(deps, prep, resolved, provider_config, &mut requested)?;
    logging::log_call(log, prep, resolved, &filtered)?;
    let parts = request_parts(prep, resolved, provider_config, &filtered, timeout)?;
    let mode = if prep.is_async.0 {
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
        input: &parts.input,
        optional_params: &parts.optional_params,
        custom_llm_provider: &resolved.provider,
        params: &parts.params,
        extra_headers: prep.extra_headers.as_ref(),
        extra_body: prep.call.extra_body.as_ref(),
        timeout: Some(parts.seconds),
        fake_stream: Some(&parts.fake),
        litellm_metadata: parts.metadata.as_ref(),
        shared_session: prep.call.shared_session,
    };
    let outcome = run(&http, mode, &create_call, &deps.hooks, log).await?;
    finish(outcome, resolved, parts.metadata.as_ref())
}
fn filtered_params<S, F, D, E, Context>(
    deps: &ResponsesCreateDeps<'_, S, F, D, E, Context>,
    prep: &Prepared<'_>,
    resolved: &Resolved,
    provider: &F::Provider,
    requested: &mut Params,
) -> Result<Params, CreateError>
where
    S: ClientSource,
    F: CreateConfigFactory,
    D: ?Sized,
    E: ?Sized,
    Context: Send,
{
    let call_drop = prep.kwargs.get("drop_params").map(decision_json);
    optional_params_responses(
        provider,
        &resolved.model,
        requested,
        &prep.call.allowed_openai_params.clone().unwrap_or_default(),
        DropPolicy {
            global: deps.drop_params,
            call: normalize_drop_params(call_drop.as_ref()),
        },
    )
    .map_err(CreateError::from)
}
fn request_parts<Provider>(
    prep: &Prepared<'_>,
    resolved: &Resolved,
    provider: &Provider,
    filtered: &Params,
    timeout: RequestTimeout,
) -> Result<RequestParts, CreateError>
where
    Provider: CreateCapabilities,
{
    let input = restore_input_item_ids(&prep.input);
    let optional_params = object_from_params(filtered);
    let params = create_params(resolved, prep)?;
    let metadata = litellm_metadata(&prep.kwargs)?;
    let call_timeout = prep.call.timeout.filter(|value| *value != 0.0_f64);
    let seconds = call_timeout.unwrap_or(timeout.seconds);
    let fake = Json::Bool(provider.should_fake_stream(
        &resolved.model,
        prep.call.stream,
        &resolved.provider,
    ));
    Ok(RequestParts {
        input,
        optional_params,
        params,
        metadata,
        seconds,
        fake,
    })
}
pub(super) async fn bridge<S, F, D, E, Context>(
    deps: &ResponsesCreateDeps<'_, S, F, D, E, Context>,
    prep: &Prepared<'_>,
    config: Option<&ResponsesApiConfig>,
    requested: &Params,
    context: Context,
) -> Result<Created<Context>, CreateError>
where
    S: ClientSource<Client: CreateTransport>,
    F: CreateConfigFactory,
    D: DeploymentHookDispatcher + ?Sized,
    E: ContinuationExecutor + ?Sized,
    Context: Send,
{
    let mut kwargs = bridge_kwargs(
        &prep.kwargs,
        config,
        &prep.call.allowed_openai_params.clone().unwrap_or_default(),
    );
    let timeout = RequestTimeout::resolve(deps.environment)?;
    let seconds = prep.call.timeout.unwrap_or(timeout.seconds);
    let Some(value) = Number::from_f64(seconds) else {
        return Err(CreateError::InvalidTimeout { seconds });
    };
    kwargs.insert(String::from("timeout"), JsonValue::Number(value));
    kwargs.insert(
        String::from("extra_body"),
        prep.call
            .extra_body
            .as_ref()
            .map_or(JsonValue::Null, object_value),
    );
    let allowed = prep
        .call
        .allowed_openai_params
        .as_ref()
        .map_or(JsonValue::Null, |names| {
            JsonValue::Array(names.iter().cloned().map(JsonValue::Text).collect())
        });
    kwargs.insert(String::from("allowed_openai_params"), allowed);
    let request = object_from_params(requested);
    let bridge_call = BridgeCall {
        model: &prep.model,
        input: &prep.input,
        request: &request,
        custom_llm_provider: prep.provider.as_deref(),
        is_async: prep.is_async.0,
        stream: prep.call.stream,
        extra_headers: prep.extra_headers.as_ref(),
        kwargs: &kwargs,
    };
    Ok(deps.bridge.respond(&bridge_call, context).await?)
}
fn finish<Context>(
    outcome: CreateOutcome,
    resolved: &Resolved,
    metadata: Option<&HttpParams>,
) -> Result<Created<Context>, CreateError> {
    match outcome {
        CreateOutcome::Value(raw) => {
            let Ok(mut response) = serde_json::from_str::<ResponsesApiResponse>(raw.get()) else {
                return Ok(Created::Raw(raw));
            };
            let affinity: AffinityMetadata =
                metadata.map_or_else(|| Ok(AffinityMetadata::default()), transcode)?;
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
        CreateOutcome::Live(live) => Ok(Created::Live(live)),
        CreateOutcome::Synthetic(synthetic) => Ok(Created::Synthetic(synthetic)),
    }
}
fn create_params(resolved: &Resolved, prep: &Prepared<'_>) -> Result<CreateParams, CreateError> {
    let mut map: HttpParams = transcode(&prep.kwargs)?;
    if let Some(base) = &resolved.api_base {
        map.insert(String::from("api_base"), Json::Text(base.clone()));
    }
    if let Some(key) = &resolved.api_key {
        let secret = Json::Text(key.expose_secret().to_owned());
        map.insert(String::from("api_key"), secret);
    }
    if let Some(verify) = prep.call.ssl_verify {
        map.insert(String::from("ssl_verify"), Json::Bool(verify));
    }
    CreateParams::from_map(map).map_err(CreateError::from)
}
fn litellm_metadata(kwargs: &ParameterMap) -> Result<Option<HttpParams>, CreateError> {
    match kwargs.get("litellm_metadata") {
        None => Ok(Some(HttpParams::new())),
        Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Object(fields)) => transcode(fields).map(Some).map_err(CreateError::from),
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Text(_) | JsonValue::Array(_),
        ) => Err(CreateError::Field {
            field: "litellm_metadata",
            expected: "an object",
        }),
    }
}
