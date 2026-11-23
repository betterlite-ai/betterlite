use std::collections::BTreeMap;

use betterlite_models::controls::json_value::JsonValue;
use secrecy::ExposeSecret as _;

use crate::api::responses_create_http::config::Headers;
use crate::api::responses_parameters::forwarded_headers::merge_forwarded_headers;
use crate::api::responses_provider_selection::ParameterMap;

use super::call::ResponsesCreateCall;
use super::error::CreateError;

// dict(litellm_params): the caller's kwargs, with the connection values responses() names itself
pub(super) fn connection_kwargs(call: &ResponsesCreateCall) -> ParameterMap {
    let mut kwargs = call.kwargs.clone();
    if let Some(base) = &call.api_base {
        kwargs.insert(String::from("api_base"), JsonValue::Text(base.clone()));
    }
    if let Some(key) = &call.api_key {
        kwargs.insert(
            String::from("api_key"),
            JsonValue::Text(key.expose_secret().to_owned()),
        );
    }
    if let Some(verify) = call.ssl_verify {
        kwargs.insert(String::from("ssl_verify"), JsonValue::Bool(verify));
    }
    kwargs
}

// merge_client_forwarded_headers: the client's headers lose, case-insensitively, to the caller's extra headers
pub(super) fn merged_headers(
    call: &ResponsesCreateCall,
    kwargs: &ParameterMap,
) -> Result<Option<Headers>, CreateError> {
    let explicit = call.extra_headers.as_ref().map(|headers| {
        headers
            .iter()
            .map(|(name, value)| (name.clone(), JsonValue::Text(value.clone())))
            .collect::<BTreeMap<String, JsonValue>>()
    });
    let merged = merge_forwarded_headers(explicit, client_headers(kwargs)?);
    merged
        .map(|headers| {
            headers
                .into_iter()
                .map(|(name, value)| match value {
                    JsonValue::Text(text) => Ok((name, text)),
                    JsonValue::Null
                    | JsonValue::Bool(_)
                    | JsonValue::Number(_)
                    | JsonValue::Array(_)
                    | JsonValue::Object(_) => Err(CreateError::Field {
                        field: "headers",
                        expected: "an object of strings",
                    }),
                })
                .collect::<Result<Headers, CreateError>>()
        })
        .transpose()
}

fn client_headers(kwargs: &ParameterMap) -> Result<Option<BTreeMap<String, String>>, CreateError> {
    match kwargs.get("headers") {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Object(fields)) => fields
            .iter()
            .map(|(name, value)| match value {
                JsonValue::Text(text) => Ok((name.clone(), text.clone())),
                JsonValue::Null
                | JsonValue::Bool(_)
                | JsonValue::Number(_)
                | JsonValue::Array(_)
                | JsonValue::Object(_) => Err(CreateError::Field {
                    field: "headers",
                    expected: "an object of strings",
                }),
            })
            .collect::<Result<BTreeMap<String, String>, CreateError>>()
            .map(Some),
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Text(_) | JsonValue::Array(_),
        ) => Err(CreateError::Field {
            field: "headers",
            expected: "an object",
        }),
    }
}
