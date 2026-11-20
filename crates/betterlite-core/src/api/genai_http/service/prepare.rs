use std::time::Duration;

use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions::Json as DecisionJson;
use betterlite_prompts::arguments::json::{Json, Object};
use http::HeaderMap as HttpHeaders;
use http::header::{HeaderName, HeaderValue};
use indexmap::IndexMap;
use url::Url;

use super::super::config::{AuthCall, GenerateContentHttpConfig};
use super::super::error::{GenAiHttpError, wire_failure};
use super::super::stream::{StreamInputs, decision_json, decision_object};
use super::{
    ClientSource, GenAiRequest, GenerateContentCall, GenerateContentDeps, Prepared, Selected,
};
use crate::api::vector_store_http_write::service::{ClientKind, LogContext};
use crate::genai_setup::config::GenerateContentRequest;

// The handler's front half: client selection, auth, both overlays, the pre-call and the wire form of the request
pub(super) async fn prepare<'borrow, 'deps, 'call, C, S>(
    deps: &'borrow GenerateContentDeps<'deps, C, S>,
    mode: ClientKind,
    call: &'call GenerateContentCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<Prepared<'borrow, 'call, C, S>, GenAiHttpError<C::Error>>
where
    'deps: 'borrow,
    C: GenerateContentHttpConfig,
    S: ClientSource,
{
    let client = select_client(deps, mode, call.litellm_params.ssl_verify.as_ref())?;
    let auth = deps
        .config
        .auth_token_and_url(&AuthCall {
            api_base: call.litellm_params.api_base.as_deref(),
            model: call.model,
            params: call.litellm_params,
            stream: call.stream,
        })
        .await
        .map_err(GenAiHttpError::Auth)?;
    let headers = overlay_headers(auth.headers, call.extra_headers);
    let body = request_body(deps.config, call)?;
    log_pre_call(call, &headers, &body, &auth.url, log)?;
    let request = wire_request(deps.config, &auth.url, headers, &body, call.timeout)?;
    let inputs = StreamInputs {
        model: call.model,
        provider: call.custom_llm_provider,
        body,
        params: call.litellm_params,
        api_base: auth.url,
    };
    Ok(Prepared {
        config: deps.config,
        client,
        mode,
        stream: call.stream,
        request,
        inputs,
        settings: &deps.stream,
    })
}

fn select_client<'borrow, 'deps, C, S>(
    deps: &'borrow GenerateContentDeps<'deps, C, S>,
    mode: ClientKind,
    ssl_verify: Option<&Json>,
) -> Result<Selected<'borrow, S::Client>, GenAiHttpError<C::Error>>
where
    'deps: 'borrow,
    C: GenerateContentHttpConfig,
    S: ClientSource,
{
    if let Some(supplied) = &deps.supplied
        && supplied.kind == mode
    {
        return Ok(Selected::Supplied(supplied.transport));
    }
    Ok(Selected::Created(deps.source.client(ssl_verify)?))
}

// headers.update(extra_headers), so a caller's header beats the provider's
fn overlay_headers(
    mut headers: IndexMap<String, String>,
    extra: Option<&IndexMap<String, String>>,
) -> IndexMap<String, String> {
    if let Some(extra) = extra {
        headers.extend(extra.clone());
    }
    headers
}

// transform_generate_content_request, then data.update(extra_body)
fn request_body<C>(
    config: &C,
    call: &GenerateContentCall<'_>,
) -> Result<Object, GenAiHttpError<C::Error>>
where
    C: GenerateContentHttpConfig,
{
    let transformed = config
        .transform_request(&GenerateContentRequest {
            model: call.model,
            contents: call.contents,
            tools: call.tools,
            generation_config: call.config,
            system_instruction: call.system_instruction,
        })
        .map_err(GenAiHttpError::Transform)?;
    let Json::Object(mut body) = transformed else {
        return Err(GenAiHttpError::BodyNotObject);
    };
    if let Some(extra) = call.extra_body {
        body.extend(extra.clone());
    }
    Ok(body)
}

fn log_pre_call(
    call: &GenerateContentCall<'_>,
    headers: &IndexMap<String, String>,
    body: &Object,
    api_base: &str,
    log: &mut LogContext<'_, '_>,
) -> Result<(), PrePostError> {
    let mut logged = IndexMap::new();
    for (name, value) in headers {
        logged.insert(name.clone(), DecisionJson::Text(value.clone()));
    }
    let record = PreCall {
        input: decision_json(call.contents),
        model: None,
        additional_args: AdditionalArgs {
            api_base: Some(api_base.to_owned()),
            headers: Some(logged),
            complete_input_dict: Some(RawBody::Object(decision_object(body))),
            request_str: None,
            openai_sdk: None,
            passthrough: IndexMap::new(),
        },
    };
    pre_call(log.state, &record, log.env).map(drop)
}

// The request as the wire carries it: the provider's URL, the overlaid headers and the encoded body
fn wire_request<C>(
    config: &C,
    endpoint: &str,
    headers: IndexMap<String, String>,
    body: &Object,
    timeout: Option<Duration>,
) -> Result<GenAiRequest, GenAiHttpError<C::Error>>
where
    C: GenerateContentHttpConfig,
{
    let url = Url::parse(endpoint).map_err(|source| {
        GenAiHttpError::Provider(wire_failure(
            config,
            format!("the request URL {endpoint} cannot be parsed: {source}"),
        ))
    })?;
    Ok(GenAiRequest {
        url,
        headers: wire_headers(config, headers)?,
        body: encode_body(config, body)?,
        timeout,
    })
}

fn wire_headers<C>(
    config: &C,
    headers: IndexMap<String, String>,
) -> Result<HttpHeaders, GenAiHttpError<C::Error>>
where
    C: GenerateContentHttpConfig,
{
    let mut map = HttpHeaders::new();
    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|source| {
            GenAiHttpError::Provider(wire_failure(
                config,
                format!("the {name} header name is not a header name: {source}"),
            ))
        })?;
        let header_value = HeaderValue::from_str(&value).map_err(|source| {
            GenAiHttpError::Provider(wire_failure(
                config,
                format!("the {name} header value is not a header value: {source}"),
            ))
        })?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}

fn encode_body<C>(config: &C, body: &Object) -> Result<Vec<u8>, GenAiHttpError<C::Error>>
where
    C: GenerateContentHttpConfig,
{
    serde_json::to_vec(body).map_err(|source| {
        GenAiHttpError::Provider(wire_failure(
            config,
            format!("the request body cannot be written as JSON: {source}"),
        ))
    })
}
