use std::collections::BTreeMap;

use betterlite_core::api::search_providers::nimble::NimbleSearch;
use betterlite_core::api::search_providers::nimble::error::NimbleSearchError;
use betterlite_core::api::search_providers::parallel_ai::ParallelAiSearch;
use betterlite_core::api::search_providers::parallel_ai::request::PARALLEL_AI_USAGE_PARAM;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::provider::{ProviderResponse, SearchProvider as _, SearchQuery};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    provider: Provider,
    calls: Vec<Call>,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Provider {
    Nimble,
    ParallelAi,
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
enum Call {
    Url {
        #[serde(default)]
        api_base: Option<String>,
        #[serde(default)]
        environment: BTreeMap<String, String>,
    },
    Request {
        query: SearchQuery,
        #[serde(default)]
        optional_params: Object,
        #[serde(default)]
        environment: BTreeMap<String, String>,
    },
    Reply {
        #[serde(default = "default_status")]
        status: u16,
        #[serde(default)]
        headers: Headers,
        body: Value,
        #[serde(default)]
        environment: BTreeMap<String, String>,
    },
    ErrorClass {
        message: String,
        status: u16,
        #[serde(default)]
        headers: Headers,
    },
}

const fn default_status() -> u16 {
    200
}

#[derive(Serialize)]
struct Outcome {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<u16>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let outcomes: Vec<Outcome> = input
            .calls
            .iter()
            .map(|call| outcome(input.provider, call))
            .collect();
        serde_json::to_value(outcomes).map_err(failure)
    }
}

fn outcome(provider: Provider, call: &Call) -> Outcome {
    let settled = match call {
        Call::Url {
            api_base,
            environment,
        } => url(provider, api_base.as_deref(), environment),
        Call::Request {
            query,
            optional_params,
            environment,
        } => request(provider, query, optional_params, environment),
        Call::Reply {
            status,
            headers,
            body,
            environment,
        } => reply(provider, *status, headers, body, environment),
        Call::ErrorClass {
            message,
            status,
            headers,
        } => error_class(provider, message, *status, headers),
    };
    match settled {
        Ok(value) => Outcome {
            ok: true,
            value: Some(value),
            status: None,
        },
        Err(status) => Outcome {
            ok: false,
            value: None,
            status,
        },
    }
}

fn url(
    provider: Provider,
    api_base: Option<&str>,
    environment: &BTreeMap<String, String>,
) -> Result<Value, Option<u16>> {
    let endpoint = match provider {
        Provider::Nimble => NimbleSearch::load(environment)
            .complete_url(api_base, &Object::new(), None)
            .map_err(|_error| None)?,
        Provider::ParallelAi => ParallelAiSearch::load(environment)
            .complete_url(api_base, &Object::new(), None)
            .map_err(|_error| None)?,
    };
    Ok(Value::String(endpoint))
}

fn request(
    provider: Provider,
    query: &SearchQuery,
    optional_params: &Object,
    environment: &BTreeMap<String, String>,
) -> Result<Value, Option<u16>> {
    match provider {
        Provider::Nimble => {
            let body = NimbleSearch::load(environment)
                .transform_search_request(query, optional_params)
                .map_err(|_error| None)?;
            serde_json::to_value(body).map_err(|_error| None)
        }
        Provider::ParallelAi => {
            let body = ParallelAiSearch::load(environment)
                .transform_search_request(query, optional_params)
                .map_err(|_error| None)?;
            serde_json::to_value(body).map_err(|_error| None)
        }
    }
}

fn reply(
    provider: Provider,
    status: u16,
    headers: &Headers,
    body: &Value,
    environment: &BTreeMap<String, String>,
) -> Result<Value, Option<u16>> {
    let text = serde_json::to_string(body).map_err(|_error| None)?;
    let response = ProviderResponse {
        status_code: status,
        headers,
        text: &text,
    };
    match provider {
        Provider::Nimble => {
            let parsed = NimbleSearch::load(environment)
                .transform_search_response(&response)
                .map_err(|error| match error {
                    NimbleSearchError::Provider(error) => Some(error.status_code),
                    _ => None,
                })?;
            serde_json::to_value(parsed).map_err(|_error| None)
        }
        Provider::ParallelAi => {
            let parsed = ParallelAiSearch::load(environment)
                .transform_search_response(&response)
                .map_err(|_error| None)?;
            let usage = parsed.hidden_params.get(PARALLEL_AI_USAGE_PARAM).cloned();
            let encoded = serde_json::to_value(parsed).map_err(|_error| None)?;
            // LiteLLM always writes the key, so an absent one must read as a different answer, not as null
            let answer = match usage {
                Some(present) => serde_json::json!({"reply": encoded, "usage": present}),
                None => serde_json::json!({"reply": encoded}),
            };
            Ok(answer)
        }
    }
}

fn error_class(
    provider: Provider,
    message: &str,
    status: u16,
    headers: &Headers,
) -> Result<Value, Option<u16>> {
    let empty: BTreeMap<String, String> = BTreeMap::new();
    let error = match provider {
        Provider::Nimble => NimbleSearch::load(&empty).error_class(message, status, headers),
        Provider::ParallelAi => {
            ParallelAiSearch::load(&empty).error_class(message, status, headers)
        }
    };
    Ok(serde_json::json!({
        "status_code": error.status_code,
        "message": error.message,
    }))
}

fn failure<Error>(error: Error) -> Box<HelperError>
where
    Error: std::fmt::Display,
{
    Box::new(HelperError {
        class: "B8025".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
