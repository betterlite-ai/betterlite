use std::collections::BTreeMap;
use std::str::from_utf8;

use betterlite_http::proxy::{StreamResponse, UpstreamResponse};
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::custom::CustomPromptTemplate;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;

use super::credentials::{predibase_base, predibase_key, predibase_tenant_id};
use super::json;
use super::{CallLogging, ChatContext, CustomPromptTable, DispatchDeps, HubDispatchError};
use crate::operation::OperationError;
use crate::predibase::config::{
    CallFailure, PredibaseConfig, call_error, get_complete_url, validate_environment,
};
use crate::predibase::error::{PredibaseSetupError, get_error_class};
use crate::predibase::reply::{PredibaseResponse, ReplyInput, transform_response};
use crate::predibase::request::{CustomPrompt, RequestBody, RequestContext};

// make_call reads the whole error body into its message, and betterlite bounds how much a stream may hold
const ERROR_BODY_LIMIT: usize = 0x400_0000;

pub enum Completion {
    Reply(Box<PredibaseResponse>),
    Stream(Box<StreamResponse>),
}

// everything main.py L3936-L3968 resolves before B5846 builds the request
struct Endpoint {
    url: String,
    headers: Headers,
    options: Object,
    stream: bool,
}

fn custom_prompt(
    model: &str,
    template: &CustomPromptTemplate,
) -> Result<CustomPrompt, HubDispatchError> {
    let raw =
        serde_json::to_value(template).map_err(|source| HubDispatchError::CustomPrompt {
            model: model.to_owned(),
            source,
        })?;
    serde_json::from_value(raw).map_err(|source| HubDispatchError::CustomPrompt {
        model: model.to_owned(),
        source,
    })
}

fn custom_prompts(
    table: &CustomPromptTable,
) -> Result<BTreeMap<String, CustomPrompt>, HubDispatchError> {
    let mut converted = BTreeMap::new();
    for (model, template) in table {
        converted.insert(model.clone(), custom_prompt(model, template)?);
    }
    Ok(converted)
}

fn logged_headers(headers: &Headers) -> IndexMap<String, Json> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect()
}

// LiteLLM's handlers read data.get("inputs", ""), so a prompt the factory left out logs as an empty string
fn logged_input(data: &RequestBody) -> Json {
    Json::Text(data.inputs.clone().unwrap_or_default())
}

fn logged_body(data: &RequestBody) -> Result<RawBody, HubDispatchError> {
    json::convert(data).map(RawBody::Object)
}

fn logged_messages(messages: &[Message]) -> Result<Json, HubDispatchError> {
    json::convert(messages)
}

fn reply_headers(headers: &http::HeaderMap) -> BTreeMap<String, String> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            let text = value.to_str().ok()?;
            Some((name.as_str().to_owned(), text.to_owned()))
        })
        .collect()
}

fn pre_call_args(url: &str, headers: &Headers, body: RawBody, acompletion: bool) -> AdditionalArgs {
    AdditionalArgs {
        api_base: Some(url.to_owned()),
        headers: Some(logged_headers(headers)),
        complete_input_dict: Some(body),
        request_str: None,
        openai_sdk: None,
        passthrough: IndexMap::from([(String::from("acompletion"), Json::Bool(acompletion))]),
    }
}

// async_streaming sets data["stream"] = True, which the request transform already took out of parameters
fn stream_body(data: &RequestBody) -> Result<String, HubDispatchError> {
    let mut fields: IndexMap<String, Json> = json::convert(data)?;
    fields.insert(String::from("stream"), Json::Bool(true));
    serde_json::to_string(&fields).map_err(|source| HubDispatchError::Encode { source })
}

fn endpoint(
    call: &mut ChatContext<'_>,
    deps: &DispatchDeps<'_>,
) -> Result<Endpoint, HubDispatchError> {
    let tenant = predibase_tenant_id(call.optional_params, deps.globals, deps.environment)?;
    let Some(tenant) = tenant else {
        return Err(HubDispatchError::PredibaseSetup(
            PredibaseSetupError::MissingTenantId,
        ));
    };
    let base = predibase_base(
        call.api_base,
        deps.globals,
        deps.environment,
        call.optional_params,
    )?;
    let key = predibase_key(call.api_key, deps.globals, deps.environment);
    let options: Object = call.optional_params.clone();
    let stream = matches!(options.get("stream"), Some(PromptJson::Bool(true)));
    let url = get_complete_url(
        Some(tenant.expose_secret()),
        base.as_deref(),
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
    let custom = custom_prompts(call.custom_prompt_dict.unwrap_or(&empty))?;
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

// main.py L3936-L3992: the tenant id and base pops first, then the module's request, call and reply
pub async fn complete(
    call: &mut ChatContext<'_>,
    deps: &DispatchDeps<'_>,
    log: &mut CallLogging<'_, '_>,
) -> Result<Completion, HubDispatchError> {
    let endpoint = endpoint(call, deps)?;
    let data = request_body(call, deps, &endpoint.options).await?;
    let args = pre_call_args(
        &endpoint.url,
        &endpoint.headers,
        logged_body(&data)?,
        call.acompletion,
    );
    pre_call(
        log.state,
        &PreCall {
            input: logged_input(&data),
            model: None,
            additional_args: args,
        },
        log.env,
    )
    .map_err(|source| HubDispatchError::PreCall { source })?;
    called(call, deps, log, &endpoint, &data).await
}

async fn called(
    call: &ChatContext<'_>,
    deps: &DispatchDeps<'_>,
    log: &mut CallLogging<'_, '_>,
    endpoint: &Endpoint,
    data: &RequestBody,
) -> Result<Completion, HubDispatchError> {
    if endpoint.stream && call.acompletion {
        return streamed(call, deps, log, endpoint, data).await;
    }
    let body = serde_json::to_vec(data).map_err(|source| HubDispatchError::Encode { source })?;
    if endpoint.stream {
        // the sync path returns the open stream without a status check or a post_call, as LiteLLM's wrapper does
        let response = deps
            .post
            .post_stream(&endpoint.url, &endpoint.headers, &body, deps.timeout)
            .await?;
        return Ok(Completion::Stream(Box::new(response)));
    }
    let response = deps
        .post
        .post(&endpoint.url, &endpoint.headers, &body, deps.timeout)
        .await?;
    replied(call, deps, log, &response, data, &endpoint.options).await
}

// make_call raises PredibaseError with the whole error body as its message
async fn stream_error(response: StreamResponse) -> HubDispatchError {
    let status = response.response.status().as_u16();
    match response.buffered(ERROR_BODY_LIMIT).await {
        Ok(buffered) => match from_utf8(&buffered.body) {
            Ok(text) => {
                HubDispatchError::Predibase(get_error_class(text, status, &BTreeMap::new()))
            }
            Err(source) => HubDispatchError::BodyNotUtf8 { source },
        },
        Err(source) => HubDispatchError::Operation(OperationError::from(source)),
    }
}

fn stream_post_call(
    log: &mut CallLogging<'_, '_>,
    call: &ChatContext<'_>,
    body: String,
) -> Result<(), HubDispatchError> {
    post_call(
        log.state,
        &PostCall {
            original_response: Json::Null,
            input: logged_messages(call.messages)?,
            additional_args: AdditionalArgs {
                api_base: None,
                headers: None,
                complete_input_dict: Some(RawBody::Text(body)),
                request_str: None,
                openai_sdk: None,
                passthrough: IndexMap::new(),
            },
        },
        log.env,
    );
    Ok(())
}

async fn streamed(
    call: &ChatContext<'_>,
    deps: &DispatchDeps<'_>,
    log: &mut CallLogging<'_, '_>,
    endpoint: &Endpoint,
    data: &RequestBody,
) -> Result<Completion, HubDispatchError> {
    let body = stream_body(data)?;
    let response = deps
        .post
        .post_stream(
            &endpoint.url,
            &endpoint.headers,
            body.as_bytes(),
            deps.timeout,
        )
        .await?;
    if response.response.status() != 200 {
        return Err(stream_error(response).await);
    }
    stream_post_call(log, call, body)?;
    Ok(Completion::Stream(Box::new(response)))
}

fn reply_post_call(
    log: &mut CallLogging<'_, '_>,
    call: &ChatContext<'_>,
    response: &UpstreamResponse,
    data: &RequestBody,
) -> Result<(), HubDispatchError> {
    post_call(
        log.state,
        &PostCall {
            original_response: json::logged_reply(&response.body),
            input: logged_messages(call.messages)?,
            additional_args: AdditionalArgs {
                api_base: None,
                headers: None,
                complete_input_dict: Some(logged_body(data)?),
                request_str: None,
                openai_sdk: None,
                passthrough: IndexMap::new(),
            },
        },
        log.env,
    );
    Ok(())
}

async fn replied(
    call: &ChatContext<'_>,
    deps: &DispatchDeps<'_>,
    log: &mut CallLogging<'_, '_>,
    response: &UpstreamResponse,
    data: &RequestBody,
    options: &Object,
) -> Result<Completion, HubDispatchError> {
    let text =
        from_utf8(&response.body).map_err(|source| HubDispatchError::BodyNotUtf8 { source })?;
    if let Some(error) = call_error(
        CallFailure::Response {
            status: response.status,
            body: text,
        },
        false,
    ) {
        return Err(HubDispatchError::Predibase(error));
    }
    reply_post_call(log, call, response, data)?;
    let reply = transform_response(
        &ReplyInput {
            body: &response.body,
            status: response.status,
            headers: &reply_headers(&response.headers),
            request_body: data,
            options,
            messages: call.messages,
            model: call.model,
            created: deps.now.timestamp(),
        },
        deps.predibase_counter,
    )
    .await?;
    Ok(Completion::Reply(Box::new(reply)))
}
