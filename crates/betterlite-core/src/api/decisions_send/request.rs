use std::sync::PoisonError;

use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions::Json;
use betterlite_openai::decisions::call::DecisionsCall;
use betterlite_openai::openai_base_config::Headers;
use betterlite_redact::secret::Redactor;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use serde_json::Error as JsonError;
use url::Url;

use super::clients::{DecisionsBody, DecisionsRequest};
use super::config::DecisionsSendConfig;
use super::error::{DecisionsSendError, plain};
use crate::api::vector_store_http_write::error::WriteError;

// _prepare_decisions_request's result: the url, the headers logging sees, and one copy of the body
pub(crate) struct Prepared {
    pub(crate) url: String,
    pub(crate) outbound_headers: Headers,
    pub(crate) body: String,
}

// validate_environment, then get_complete_url, then the body copy, then the optional pre_call, in that order
pub(crate) fn prepare(
    config: &dyn DecisionsSendConfig,
    call: &DecisionsCall<'_>,
    env: &PrePostEnv<'_>,
) -> Result<Prepared, DecisionsSendError> {
    let outbound_headers =
        config.validate_environment(&call.headers, &call.model, call.api_key.as_ref());
    let url = config.get_complete_url(&call.api_base, &call.model);
    let body = serde_json::to_string(&call.body)
        .map_err(|error| plain(call, env.redactor, Box::new(error)))?;
    let data: Json =
        serde_json::from_str(&body).map_err(|error| plain(call, env.redactor, Box::new(error)))?;
    if call.logging_obj.is_some() {
        log(call, env, &url, &outbound_headers, &data)?;
    }
    Ok(Prepared {
        url,
        outbound_headers,
        body,
    })
}

// logging_obj.pre_call(input=data, api_key=..., model=model, additional_args={...}) runs only when a
// logging object exists, and sees the headers before sign_request changed them
fn log(
    call: &DecisionsCall<'_>,
    env: &PrePostEnv<'_>,
    url: &str,
    headers: &Headers,
    data: &Json,
) -> Result<(), DecisionsSendError> {
    let Some(logging) = call.logging_obj else {
        return Ok(());
    };
    let additional_args = AdditionalArgs {
        api_base: Some(url.to_owned()),
        headers: Some(fields(headers)),
        complete_input_dict: Some(
            raw_body(data).map_err(|error| plain(call, env.redactor, Box::new(error)))?,
        ),
        request_str: None,
        openai_sdk: None,
        passthrough: IndexMap::new(),
    };
    let pre = PreCall {
        input: data.clone(),
        model: Some(call.model.clone()),
        additional_args,
    };
    let mut state = logging.lock().unwrap_or_else(PoisonError::into_inner);
    pre_call(&mut state, &pre, env).map_err(|error| plain(call, env.redactor, Box::new(error)))
}

// The POST both handlers send: signed headers, the signed bytes when the config built them, else the JSON copy
pub(crate) fn wire(
    call: &DecisionsCall<'_>,
    redactor: &Redactor,
    prepared: &Prepared,
    signed: (Headers, Option<Vec<u8>>),
) -> Result<DecisionsRequest, DecisionsSendError> {
    let (signed_headers, signed_body) = signed;
    let mut headers = HeaderMap::new();
    for (name, value) in signed_headers {
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|source| {
            plain(
                call,
                redactor,
                Box::new(WriteError::HeaderName {
                    name: name.clone(),
                    source,
                }),
            )
        })?;
        let header_value = HeaderValue::from_str(&value).map_err(|source| {
            plain(
                call,
                redactor,
                Box::new(WriteError::HeaderValue { name, source }),
            )
        })?;
        headers.insert(header_name, header_value);
    }
    let url = Url::parse(&prepared.url).map_err(|source| {
        plain(
            call,
            redactor,
            Box::new(WriteError::RequestUrl {
                url: prepared.url.clone(),
                source,
            }),
        )
    })?;
    let body = match signed_body {
        Some(bytes) => DecisionsBody::Signed(bytes),
        None => DecisionsBody::Json(prepared.body.clone().into_bytes()),
    };
    Ok(DecisionsRequest {
        url,
        headers,
        body,
        timeout: call.timeout,
    })
}

fn fields(headers: &Headers) -> IndexMap<String, Json> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect()
}

// The data is a copy of the transformed body, so the logging record and the sent JSON are the same map
fn raw_body(data: &Json) -> Result<RawBody, JsonError> {
    match data {
        Json::Object(entries) => Ok(RawBody::Object(entries.clone())),
        other => Ok(RawBody::Text(serde_json::to_string(other)?)),
    }
}
