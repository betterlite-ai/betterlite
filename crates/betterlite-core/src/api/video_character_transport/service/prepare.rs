use std::collections::BTreeMap;

use betterlite_http::interactions::base::{ClientError, Params, RequestHeaders};
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use reqwest::Method;
use secrecy::{ExposeSecret as _, SecretString};

use super::super::clients::{CharacterBody, CharacterRequest};
use super::super::config::{CharacterConfig, CharacterContext, CharacterEnvironment};
use super::super::error::{CharacterError, CharacterFault, converted};
use super::wire::{header_map, multipart_body, request_url};
use super::{CharacterCreateCall, CharacterGetCall};

// the litellm_params fields the handlers read before a transform sees them
struct CallParts<'call> {
    extra_headers: Option<&'call BTreeMap<String, String>>,
    litellm_params: &'call Params,
    api_key: Option<&'call SecretString>,
}

struct PreCallRecord<'call> {
    input: &'call str,
    url: &'call str,
    body: Option<IndexMap<String, Json>>,
}

pub(crate) fn prepare_create<C>(
    config: &C,
    logging: &mut LoggingState,
    env: &PrePostEnv<'_>,
    boundary: &str,
    call: &CharacterCreateCall<'_>,
) -> Result<CharacterRequest, CharacterFault<C>>
where
    C: CharacterConfig,
{
    let parts = CallParts {
        extra_headers: call.extra_headers,
        litellm_params: call.litellm_params,
        api_key: call.api_key,
    };
    let (headers, base) = validated(config, &parts)?;
    let (url_text, files) = config
        .transform_create_request(
            call.name,
            call.video,
            CharacterContext {
                api_base: &base,
                litellm_params: call.litellm_params,
                headers: &headers,
            },
        )
        .map_err(CharacterError::Provider)?;
    let named = IndexMap::from([("name".to_owned(), Json::Text(call.name.to_owned()))]);
    log_pre_call(
        logging,
        env,
        PreCallRecord {
            input: call.name,
            url: &url_text,
            body: Some(named),
        },
        &headers,
    )
    .map_err(CharacterError::Logging)?;
    Ok(CharacterRequest {
        method: Method::POST,
        url: request_url(&url_text, &[]).map_err(|failure| converted(config, failure))?,
        headers: header_map(&headers).map_err(|failure| converted(config, failure))?,
        body: multipart_body(&files, boundary).map_err(CharacterError::Multipart)?,
        timeout: call.timeout,
    })
}

pub(crate) fn prepare_get<C>(
    config: &C,
    logging: &mut LoggingState,
    env: &PrePostEnv<'_>,
    call: &CharacterGetCall<'_>,
) -> Result<CharacterRequest, CharacterFault<C>>
where
    C: CharacterConfig,
{
    let parts = CallParts {
        extra_headers: call.extra_headers,
        litellm_params: call.litellm_params,
        api_key: call.api_key,
    };
    let (headers, base) = validated(config, &parts)?;
    let (url_text, query) = config
        .transform_get_request(
            call.character_id,
            CharacterContext {
                api_base: &base,
                litellm_params: call.litellm_params,
                headers: &headers,
            },
        )
        .map_err(CharacterError::Provider)?;
    log_pre_call(
        logging,
        env,
        PreCallRecord {
            input: call.character_id,
            url: &url_text,
            body: None,
        },
        &headers,
    )
    .map_err(CharacterError::Logging)?;
    Ok(CharacterRequest {
        method: Method::GET,
        url: request_url(&url_text, &query).map_err(|failure| converted(config, failure))?,
        headers: header_map(&headers).map_err(|failure| converted(config, failure))?,
        body: CharacterBody::Empty,
        timeout: None,
    })
}

// validate_environment, then the caller's headers applied over its answer, then get_complete_url
fn validated<C>(
    config: &C,
    call: &CallParts<'_>,
) -> Result<(RequestHeaders, String), CharacterFault<C>>
where
    C: CharacterConfig,
{
    let key = api_key(call.api_key, call.litellm_params).map_err(CharacterError::Client)?;
    let mut caller = RequestHeaders::new();
    for (name, value) in call.extra_headers.into_iter().flatten() {
        caller.insert(name.clone(), SecretString::from(value.clone()));
    }
    let mut headers = config.validate_environment(CharacterEnvironment {
        api_key: key.as_ref(),
        headers: caller.clone(),
        model: "",
        litellm_params: call.litellm_params,
    });
    headers.extend(caller);
    let base = api_base(call.litellm_params).map_err(CharacterError::Client)?;
    let url = config
        .complete_url("", base, call.litellm_params)
        .map_err(CharacterError::Provider)?;
    Ok((headers, url))
}

// api_key or litellm_params.api_key, with the caller's value winning when it is there
fn api_key(
    caller: Option<&SecretString>,
    params: &Params,
) -> Result<Option<SecretString>, ClientError> {
    if caller.is_some() {
        return Ok(caller.cloned());
    }
    let fallback = match params.get("api_key") {
        None | Some(JsonValue::Null) => None,
        Some(JsonValue::Text(value)) => Some(SecretString::from(value.clone())),
        Some(_) => {
            return Err(ClientError::FieldType {
                field: "api_key",
                expected: "a string",
            });
        }
    };
    Ok(fallback)
}

// litellm_params.get("api_base"), where absent and null mean none and an explicit empty string stays empty
fn api_base(params: &Params) -> Result<Option<&str>, ClientError> {
    match params.get("api_base") {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Text(value)) => Ok(Some(value)),
        Some(_) => Err(ClientError::FieldType {
            field: "api_base",
            expected: "a string",
        }),
    }
}

fn log_pre_call(
    state: &mut LoggingState,
    env: &PrePostEnv<'_>,
    record: PreCallRecord<'_>,
    headers: &RequestHeaders,
) -> Result<(), PrePostError> {
    let mut logged = IndexMap::new();
    for (name, value) in headers {
        logged.insert(name.clone(), Json::Text(value.expose_secret().to_owned()));
    }
    let call = PreCall {
        input: Json::Text(record.input.to_owned()),
        model: None,
        additional_args: AdditionalArgs {
            api_base: Some(record.url.to_owned()),
            headers: Some(logged),
            complete_input_dict: record.body.map(RawBody::Object),
            request_str: None,
            openai_sdk: None,
            passthrough: IndexMap::new(),
        },
    };
    pre_call(state, &call, env).map(drop)
}
