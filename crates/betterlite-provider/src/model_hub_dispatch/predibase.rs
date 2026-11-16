use std::collections::BTreeMap;
use std::str::from_utf8;

use betterlite_http::proxy::{StreamResponse, UpstreamResponse};
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;

use super::credentials::{predibase_base, predibase_key, predibase_tenant_id};
use super::hub::headers_from;
use super::json;
use super::{CallLogging, ChatContext, CustomPromptTable, DispatchDeps, HubDispatchError};
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

fn log_headers(headers: &Headers) -> IndexMap<String, Json> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect()
}

fn args(
    api_base: Option<String>,
    headers: Option<IndexMap<String, Json>>,
    body: Option<RawBody>,
    passthrough: IndexMap<String, Json>,
) -> AdditionalArgs {
    AdditionalArgs {
        api_base,
        headers,
        complete_input_dict: body,
        request_str: None,
        openai_sdk: None,
        passthrough,
    }
}

fn stream_body(data: &RequestBody) -> Result<String, HubDispatchError> {
    let mut fields: IndexMap<String, Json> = json::convert(data)?;
    fields.insert(String::from("stream"), Json::Bool(true));
    serde_json::to_string(&fields).map_err(|source| HubDispatchError::Encode { source })
}

fn endpoint(
    call: &mut ChatContext<'_>,
    deps: &DispatchDeps<'_>,
) -> Result<Endpoint, HubDispatchError> {
    let Some(tenant) = predibase_tenant_id(call.optional_params, deps.globals, deps.environment)?
    else {
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

pub async fn complete(
    call: &mut ChatContext<'_>,
    deps: &DispatchDeps<'_>,
    log: &mut CallLogging<'_, '_>,
) -> Result<Completion, HubDispatchError> {
    let endpoint = endpoint(call, deps)?;
    let data = request_body(call, deps, &endpoint.options).await?;
    pre_call(
        log.state,
        &PreCall {
            input: Json::Text(data.inputs.clone().unwrap_or_default()),
            model: None,
            additional_args: args(
                Some(endpoint.url.clone()),
                Some(log_headers(&endpoint.headers)),
                Some(json::convert(&data).map(RawBody::Object)?),
                IndexMap::from([(String::from("acompletion"), Json::Bool(call.acompletion))]),
            ),
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
    replied(call, deps, log, &response, data).await
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
    post_call(
        log.state,
        &PostCall {
            original_response: Json::Null,
            input: json::convert(call.messages)?,
            additional_args: args(None, None, Some(RawBody::Text(body)), IndexMap::new()),
        },
        log.env,
    );
    Ok(Completion::Stream(Box::new(response)))
}

async fn replied(
    call: &ChatContext<'_>,
    deps: &DispatchDeps<'_>,
    log: &mut CallLogging<'_, '_>,
    response: &UpstreamResponse,
    data: &RequestBody,
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
    let logged = json::convert(data).map(RawBody::Object)?;
    post_call(
        log.state,
        &PostCall {
            original_response: json::logged_reply(&response.body),
            input: json::convert(call.messages)?,
            additional_args: args(None, None, Some(logged), IndexMap::new()),
        },
        log.env,
    );
    let reply = transform_response(
        &ReplyInput {
            body: &response.body,
            status: response.status,
            headers: &headers_from(&response.headers),
            request_body: data,
            options: call.optional_params,
            messages: call.messages,
            model: call.model,
            created: deps.now.timestamp(),
        },
        deps.predibase_counter,
    )
    .await?;
    Ok(Completion::Reply(Box::new(reply)))
}
