use std::collections::BTreeMap;
use std::str::from_utf8;

use betterlite_http::proxy::{StreamResponse, UpstreamResponse};
use betterlite_logging::logging_pre_post::args::RawBody;
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use http::Method;
use indexmap::IndexMap;
use secrecy::ExposeSecret;

use super::credentials::{PREDIBASE_BASE, PREDIBASE_KEY, PREDIBASE_TENANT_ID, Sources, resolve};
use super::hub::headers_from;
use super::json::{self, additional_args, header_values, log_opened_stream};
use super::transport::WireRequest;
use super::{ChatContext, CustomPromptTable, DispatchDeps, HubDispatchError};
use crate::operation::OperationError;
use crate::predibase::config::{
    CallFailure, PredibaseConfig, call_error, get_complete_url, validate_environment,
};
use crate::predibase::error::{PredibaseSetupError, get_error_class};
use crate::predibase::reply::{PredibaseResponse, ReplyInput, transform_response};
use crate::predibase::request::{CustomPrompt, RequestBody, RequestContext};

const ERROR_BODY_LIMIT: usize = 0x400_0000;

pub enum Completion {
    Reply(Box<PredibaseResponse>),
    Stream(Box<StreamResponse>),
}

struct Endpoint {
    url: String,
    headers: Headers,
    options: Object,
    stream: bool,
}

fn custom_prompts(table: &CustomPromptTable) -> BTreeMap<String, CustomPrompt> {
    table
        .iter()
        .map(|(model, template)| {
            let prompt = CustomPrompt {
                roles: Some(template.roles.clone()),
                initial_prompt_value: Some(template.initial_prompt_value.clone()),
                final_prompt_value: Some(template.final_prompt_value.clone()),
            };
            (model.clone(), prompt)
        })
        .collect()
}

// the async stream posts data with stream=true; the sync stream and the plain call post data as is
fn encoded_body(data: &RequestBody, stream_flag: bool) -> Result<String, HubDispatchError> {
    let mut fields: IndexMap<String, Json> = json::convert(data)?;
    if stream_flag {
        fields.insert(String::from("stream"), Json::Bool(true));
    }
    serde_json::to_string(&fields).map_err(|source| HubDispatchError::Encode { source })
}

fn endpoint(
    call: &mut ChatContext<'_>,
    deps: &DispatchDeps<'_>,
) -> Result<Endpoint, HubDispatchError> {
    let lookups = Sources {
        call: None,
        globals: deps.globals,
        env: deps.environment,
    };
    let missing = HubDispatchError::PredibaseSetup(PredibaseSetupError::MissingTenantId);
    let tenant = resolve(PREDIBASE_TENANT_ID, &lookups, call.optional_params)?.ok_or(missing)?;
    let bases = Sources {
        call: call.api_base,
        ..lookups
    };
    let base = resolve(PREDIBASE_BASE, &bases, call.optional_params)?;
    let keys = Sources {
        call: call.api_key.map(ExposeSecret::expose_secret),
        ..bases
    };
    let key = resolve(PREDIBASE_KEY, &keys, &mut Object::new())?;
    let options: Object = call.optional_params.clone();
    let stream = matches!(options.get("stream"), Some(PromptJson::Bool(true)));
    let url = get_complete_url(
        Some(tenant.expose_secret()),
        base.as_ref().map(ExposeSecret::expose_secret),
        call.model,
        &options,
        Some(stream),
    )?;
    let headers = validate_environment(key.as_ref(), &BTreeMap::new())?;
    Ok(Endpoint {
        url,
        headers,
        options,
        stream,
    })
}

async fn request_body(
    call: &ChatContext<'_>,
    deps: &DispatchDeps<'_>,
    options: &Object,
) -> Result<RequestBody, HubDispatchError> {
    let empty = CustomPromptTable::new();
    let custom = custom_prompts(call.custom_prompt_dict.unwrap_or(&empty));
    let config = PredibaseConfig {
        details: true,
        max_new_tokens: deps.default_max_tokens,
        return_full_text: false,
    };
    let context = RequestContext {
        config: &config,
        prompts: deps.prompts,
        custom_prompts: &custom,
        now: deps.now,
    };
    let data = context
        .transform_request(call.model, call.messages, options)
        .await?;
    Ok(data)
}

async fn stream_error(response: StreamResponse) -> HubDispatchError {
    let status = response.response.status().as_u16();
    let buffered = match response.buffered(ERROR_BODY_LIMIT).await {
        Ok(buffered) => buffered,
        Err(source) => return HubDispatchError::Operation(OperationError::from(source)),
    };
    from_utf8(&buffered.body).map_or_else(
        |source| HubDispatchError::BodyNotUtf8 { source },
        |text| HubDispatchError::Predibase(get_error_class(text, status, &BTreeMap::new())),
    )
}

fn log_pre_call(
    call: &ChatContext<'_>,
    log: &mut LogContext<'_, '_>,
    endpoint: &Endpoint,
    data: &RequestBody,
) -> Result<(), HubDispatchError> {
    let acompletion = IndexMap::from([(String::from("acompletion"), Json::Bool(call.acompletion))]);
    let sent = PreCall {
        input: Json::Text(data.inputs.clone().unwrap_or_default()),
        model: None,
        additional_args: additional_args(
            Some(endpoint.url.clone()),
            Some(header_values(&endpoint.headers)),
            Some(json::convert(data).map(RawBody::Object)?),
            acompletion,
        ),
    };
    pre_call(log.state, &sent, log.env).map_err(|source| HubDispatchError::PreCall { source })?;
    Ok(())
}

pub async fn complete<'call>(
    call: &'call mut ChatContext<'_>,
    deps: &'call DispatchDeps<'_>,
    log: &'call mut LogContext<'_, '_>,
) -> Result<Completion, HubDispatchError> {
    let endpoint = endpoint(call, deps)?;
    let data = request_body(call, deps, &endpoint.options).await?;
    log_pre_call(call, log, &endpoint, &data)?;
    let encoded = encoded_body(&data, endpoint.stream && call.acompletion)?;
    let request = WireRequest {
        method: Method::POST,
        url: &endpoint.url,
        query: &[],
        headers: &endpoint.headers,
        body: Some(encoded.as_bytes()),
        deadline: deps.timeout,
    };
    if !endpoint.stream {
        let response = deps.post.post(&request).await?;
        checked_reply(call, log, &response, &data)?;
        return transformed(call, deps, &response, &data).await;
    }
    let response = deps.post.post_stream(&request).await?;
    if response.response.status() != 200 {
        return Err(stream_error(response).await);
    }
    if call.acompletion {
        log_opened_stream(log, call.messages, Some(RawBody::Text(encoded)))?;
    }
    Ok(Completion::Stream(Box::new(response)))
}

// call_error raises before LiteLLM logs the post_call, which carries the request body
fn checked_reply(
    call: &ChatContext<'_>,
    log: &mut LogContext<'_, '_>,
    response: &UpstreamResponse,
    data: &RequestBody,
) -> Result<(), HubDispatchError> {
    let text =
        from_utf8(&response.body).map_err(|source| HubDispatchError::BodyNotUtf8 { source })?;
    let failure = CallFailure::Response {
        status: response.status,
        body: text,
    };
    if let Some(error) = call_error(failure, false) {
        return Err(HubDispatchError::Predibase(error));
    }
    let logged = json::convert(data).map(RawBody::Object)?;
    let answer = PostCall {
        original_response: json::logged_reply(&response.body),
        input: json::convert(call.messages.as_slice())?,
        additional_args: additional_args(None, None, Some(logged), IndexMap::new()),
    };
    post_call(log.state, &answer, log.env);
    Ok(())
}

async fn transformed(
    call: &ChatContext<'_>,
    deps: &DispatchDeps<'_>,
    response: &UpstreamResponse,
    data: &RequestBody,
) -> Result<Completion, HubDispatchError> {
    let input = ReplyInput {
        body: &response.body,
        status: response.status,
        headers: &headers_from(&response.headers),
        request_body: data,
        options: call.optional_params,
        messages: call.messages,
        model: call.model,
        created: deps.now.timestamp(),
    };
    let reply = transform_response(&input, deps.predibase_counter).await?;
    Ok(Completion::Reply(Box::new(reply)))
}
