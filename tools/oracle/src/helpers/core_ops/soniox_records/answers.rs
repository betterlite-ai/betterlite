use std::collections::BTreeMap;

use betterlite_config::types::LiteLlmParams;
use betterlite_core::api::soniox::records::SUPPORTED_OPENAI_PARAMS;
use betterlite_core::api::soniox::records::auth::{
    resolve_api_base, soniox_exception, validate_environment,
};
use betterlite_core::api::soniox::records::params::map_openai_params;
use betterlite_core::api::soniox::records::response::{
    SonioxTranscriptPayload, build_response_from_payload,
};
use betterlite_openai::openai_base_config::{Headers, Object};
use serde_json::{Value, json};

use super::input::Call;
use crate::helpers::{HelperError, HelperResult};

type Failure = Box<HelperError>;

pub(super) fn answer(call: &Call) -> HelperResult {
    match call {
        Call::SupportedParams => Ok(json!({ "ok": true, "params": SUPPORTED_OPENAI_PARAMS })),
        Call::MapParams {
            non_default_params,
            optional_params,
        } => map_answer(non_default_params, optional_params),
        Call::ValidateEnvironment {
            headers,
            api_key,
            environment,
        } => Ok(validate_answer(headers, api_key.as_deref(), environment)),
        Call::CompleteUrl {
            api_base,
            environment,
        } => Ok(url_answer(api_base.as_deref(), environment)),
        Call::Error {
            message,
            status_code,
        } => Ok(error_answer(message, *status_code)),
        Call::Response {
            payload,
            response_format,
        } => response_answer(payload, response_format.as_deref()),
    }
}

fn map_answer(non_default: &Object, optional: &Object) -> HelperResult {
    let mut options = optional.clone();
    map_openai_params(non_default, &mut options).map_err(failure)?;
    let params = serde_json::to_value(&options).map_err(failure)?;
    Ok(json!({ "ok": true, "params": params, "optional_params": options }))
}

fn validate_answer(
    headers: &Headers,
    api_key: Option<&str>,
    environment: &BTreeMap<String, String>,
) -> Value {
    let (environment_key, _) = LiteLlmParams::soniox_record_settings(environment);
    match validate_environment(headers.clone(), api_key, environment_key.as_ref()) {
        Ok(merged) => json!({ "ok": true, "headers": merged }),
        Err(error) => json!({
            "ok": false,
            "error": { "status": error.status_code, "message": error.message },
        }),
    }
}

fn url_answer(api_base: Option<&str>, environment: &BTreeMap<String, String>) -> Value {
    let (_, environment_base) = LiteLlmParams::soniox_record_settings(environment);
    let url = resolve_api_base(api_base, &environment_base);
    json!({ "ok": true, "url": url })
}

fn error_answer(message: &str, status_code: i64) -> Value {
    let error = soniox_exception(message, status_code, None);
    json!({ "status": error.status_code, "message": error.message })
}

fn response_answer(payload: &Object, response_format: Option<&str>) -> HelperResult {
    let parsed = match SonioxTranscriptPayload::try_from(payload.clone()) {
        Ok(parsed) => parsed,
        Err(_) => return Ok(json!({ "ok": false, "error": { "status": null } })),
    };
    match build_response_from_payload(&parsed, None, response_format) {
        Ok(response) => {
            let encoded = serde_json::to_value(&response).map_err(failure)?;
            Ok(json!({ "ok": true, "response": encoded }))
        }
        Err(_) => Ok(json!({ "ok": false, "error": { "status": null } })),
    }
}

fn failure<Error>(error: Error) -> Failure
where
    Error: std::fmt::Display,
{
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
