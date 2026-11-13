use std::collections::BTreeMap;

use betterlite_openai::ragflow_vector_store::create::create_request;
use betterlite_openai::ragflow_vector_store::decode::decode_ragflow_create;
use betterlite_openai::ragflow_vector_store::error::{RagflowError, ResponseHeaders};
use betterlite_openai::ragflow_vector_store::{
    RagflowEnv, endpoints, get_auth_credentials, get_complete_url, search_request, search_response,
    validate_environment,
};
use betterlite_openai::vector_stores::Headers;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use super::{AuthCall, Call, DecodeCall, EnvironmentCall, UrlCall};

pub(super) fn run_call(call: &Call) -> Value {
    match call {
        Call::AuthCredentials(input) => auth_credentials(input),
        Call::ValidateEnvironment(input) => validate(input),
        Call::CompleteUrl(input) => complete_url(input),
        Call::Endpoints => json!({"ok": true, "value": endpoints()}),
        Call::CreateRequest(input) => {
            outcome(create_request(&input.optional_params, &input.api_base))
        }
        Call::CreateResponse(input) => decode(input),
        Call::SearchRequest => outcome(search_request()),
        Call::SearchResponse => outcome(search_response()),
    }
}

fn auth_credentials(input: &AuthCall) -> Value {
    let api_key = input.api_key.as_deref().map(SecretString::from);
    outcome(
        get_auth_credentials(api_key.as_ref(), &ragflow_env(&input.env))
            .map(|headers| headers_value(&headers)),
    )
}

fn validate(input: &EnvironmentCall) -> Value {
    let api_key = input
        .litellm_params
        .as_ref()
        .and_then(|params| params.api_key.as_deref())
        .map(SecretString::from);
    outcome(
        validate_environment(
            secret_headers(&input.headers),
            api_key.as_ref(),
            &ragflow_env(&input.env),
        )
        .map(|headers| headers_value(&headers)),
    )
}

fn complete_url(input: &UrlCall) -> Value {
    let litellm_api_base = input
        .litellm_params
        .as_ref()
        .and_then(|params| params.api_base.as_deref());
    json!({
        "ok": true,
        "value": get_complete_url(
            input.api_base.as_deref(),
            litellm_api_base,
            &ragflow_env(&input.env),
        ),
    })
}

fn decode(input: &DecodeCall) -> Value {
    match decode_ragflow_create(input.status_code, &input.headers, input.body.as_bytes()) {
        Ok(response) => json!({"ok": true, "value": response}),
        Err(error) => match status_and_headers(&error) {
            Some((status, headers)) => {
                json!({"ok": true, "value": {"status_code": status, "headers": headers}})
            }
            None => json!({"ok": false}),
        },
    }
}

fn outcome<T, E>(result: Result<T, E>) -> Value
where
    T: serde::Serialize,
{
    result.map_or_else(
        |_error| json!({"ok": false}),
        |value| json!({"ok": true, "value": value}),
    )
}

const fn status_and_headers(error: &RagflowError) -> Option<(u16, &ResponseHeaders)> {
    match error {
        RagflowError::ErrorResponse {
            status, headers, ..
        }
        | RagflowError::UnreadableResponse {
            status, headers, ..
        } => Some((*status, headers)),
        RagflowError::MissingAuthKey
        | RagflowError::MissingEnvironmentKey
        | RagflowError::MissingName
        | RagflowError::ChunkMethodExclusive
        | RagflowError::MissingDatasetId
        | RagflowError::SearchUnsupported { .. } => None,
    }
}

fn ragflow_env(env: &BTreeMap<String, String>) -> RagflowEnv {
    RagflowEnv {
        api_key: env
            .get("RAGFLOW_API_KEY")
            .map(|value| SecretString::from(value.as_str())),
        api_base: env.get("RAGFLOW_API_BASE").cloned(),
    }
}

fn secret_headers(headers: &BTreeMap<String, String>) -> Headers {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), SecretString::from(value.as_str())))
        .collect()
}

fn headers_value(headers: &Headers) -> Value {
    let exposed = headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.expose_secret()))
        .collect::<BTreeMap<_, _>>();
    json!(exposed)
}
