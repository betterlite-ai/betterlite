use betterlite_models::decisions::Json;
use secrecy::SecretString;

use super::error::BadField;
use super::video_config::{Headers, Params};

// GenericLiteLLMParams.api_base, which reads as no base at all when it is absent or null
pub fn api_base(litellm_params: &Params) -> Result<Option<&str>, BadField> {
    match litellm_params.get("api_base") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text)),
        Some(_) => Err(BadField {
            field: "litellm_params.api_base",
            expected: "a string",
        }),
    }
}

// The ssl_verify _get_httpx_client and get_async_httpx_client apply to the client they build
pub fn ssl_verify(litellm_params: &Params) -> Result<Option<bool>, BadField> {
    match litellm_params.get("ssl_verify") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(BadField {
            field: "litellm_params.ssl_verify",
            expected: "a boolean",
        }),
    }
}

// api_key or litellm_params.api_key: the argument wins when given, and an absent or null map key takes no key at all
pub fn api_key(
    given: Option<&SecretString>,
    litellm_params: &Params,
) -> Result<Option<SecretString>, BadField> {
    if let Some(key) = given {
        return Ok(Some(key.clone()));
    }
    match litellm_params.get("api_key") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(SecretString::from(text.clone()))),
        Some(_) => Err(BadField {
            field: "litellm_params.api_key",
            expected: "a string",
        }),
    }
}

// video_generation_optional_request_params.extra_headers, the headers validate_environment starts from
pub fn optional_headers(optional_params: &Params) -> Result<Headers, BadField> {
    let entries = match optional_params.get("extra_headers") {
        None | Some(Json::Null) => return Ok(Headers::new()),
        Some(Json::Object(entries)) => entries,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(extra_headers_error());
        }
    };
    let mut headers = Headers::new();
    for (name, value) in entries {
        let Json::Text(text) = value else {
            return Err(extra_headers_error());
        };
        headers.insert(name.clone(), text.clone());
    }
    Ok(headers)
}

fn extra_headers_error() -> BadField {
    BadField {
        field: "video_generation_optional_request_params.extra_headers",
        expected: "a map of string to string",
    }
}
