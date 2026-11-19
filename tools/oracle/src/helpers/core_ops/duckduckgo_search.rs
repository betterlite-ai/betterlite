use std::collections::BTreeMap;

use betterlite_core::api::search_providers::duckduckgo::{
    DuckDuckGoSearchConfig, DuckDuckGoSearchError,
};
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::provider::{ProviderResponse, SearchProvider as _, SearchQuery};
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    environment: BTreeMap<String, String>,
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    Headers {
        headers: BTreeMap<String, String>,
    },
    Request {
        query: SearchQuery,
        #[serde(default)]
        optional_params: Object,
    },
    Url {
        #[serde(default)]
        api_base: Option<String>,
        query: SearchQuery,
        #[serde(default)]
        optional_params: Object,
    },
    Reply {
        query: SearchQuery,
        #[serde(default)]
        optional_params: Object,
        body: String,
    },
}

#[derive(Debug, Error)]
enum DuckDuckGoSearchAnswerError {
    #[error(transparent)]
    Search(#[from] DuckDuckGoSearchError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let config = DuckDuckGoSearchConfig::load(&input.environment);
        let mut outcomes = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            outcomes.push(outcome(answer(&config, call)));
        }
        serde_json::to_value(outcomes)
            .map(|calls| json!({"calls": calls}))
            .or_value_error()
    }
}

fn answer(
    config: &DuckDuckGoSearchConfig,
    call: &Call,
) -> Result<Value, DuckDuckGoSearchAnswerError> {
    match call {
        Call::Headers { headers } => {
            let incoming: Headers = headers.clone().into_iter().collect();
            let validated = config.validate_environment(incoming, None, None);
            Ok(serde_json::to_value(validated)?)
        }
        Call::Request {
            query,
            optional_params,
        } => Ok(serde_json::to_value(
            config.transform_search_request(query, optional_params)?,
        )?),
        Call::Url {
            api_base,
            query,
            optional_params,
        } => {
            let body = config.transform_search_request(query, optional_params)?;
            let url = config.complete_url(api_base.as_deref(), optional_params, Some(&body))?;
            Ok(url_value(&url))
        }
        Call::Reply {
            query,
            optional_params,
            body,
        } => {
            // the response decodes with the limit the request transform stored
            let _request = config.transform_search_request(query, optional_params)?;
            let headers = Headers::new();
            let response = config.transform_search_response(&ProviderResponse {
                status_code: 200,
                headers: &headers,
                text: body,
            })?;
            Ok(serde_json::to_value(response)?)
        }
    }
}

fn url_value(url: &str) -> Value {
    let split = url.split_once('?');
    let address = split.map_or(url, |(address, _)| address);
    let mut params: Vec<&str> =
        split.map_or_else(Vec::new, |(_, query)| query.split('&').collect());
    params.sort_unstable();
    json!({"address": address, "params": params})
}

fn outcome(result: Result<Value, DuckDuckGoSearchAnswerError>) -> Value {
    result.map_or_else(
        |_| json!({"ok": false}),
        |value| json!({"ok": true, "value": value}),
    )
}

mod tests;
