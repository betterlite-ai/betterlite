use std::collections::BTreeMap;

use betterlite_core::api::search_providers::brave::dates::{DateFlags, DateValue, to_yyyy_mm_dd};
use betterlite_core::api::search_providers::brave::{BraveSearchConfig, BraveSearchError};
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::provider::{
    OutboundSearch, ProviderResponse, SearchProvider as _, SearchQuery,
};
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use chrono::NaiveDate;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;

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
    Request {
        #[serde(default)]
        headers: BTreeMap<String, String>,
        api_key: Option<String>,
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
    Date {
        value: Value,
        #[serde(default)]
        dayfirst: bool,
        #[serde(default)]
        yearfirst: bool,
    },
}

#[derive(Debug, Error)]
enum BraveSearchAnswerError {
    #[error(transparent)]
    Search(#[from] BraveSearchError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let today = NaiveDate::from_ymd_opt(2024, 3, 5).unwrap_or_default();
        let config = BraveSearchConfig::load(&input.environment, Some(today));
        let mut outcomes = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            outcomes.push(outcome(answer(&config, call)));
        }
        serde_json::to_value(outcomes)
            .map(|calls| json!({"calls": calls}))
            .or_value_error()
    }
}

fn answer(config: &BraveSearchConfig, call: &Call) -> Result<Value, BraveSearchAnswerError> {
    match call {
        Call::Request {
            headers,
            api_key,
            api_base,
            query,
            optional_params,
        } => {
            let key = api_key.clone().map(SecretString::from);
            let body = config.transform_search_request(query, optional_params)?;
            let outbound = OutboundSearch {
                api_base: api_base.as_deref().unwrap_or_default(),
                api_key: key.as_ref(),
                optional_params,
                body: &body,
            };
            let incoming: Headers = headers.clone().into_iter().collect();
            let signed = config.sign_request(incoming, &outbound)?.0;
            let url = config.complete_url(api_base.as_deref(), optional_params, Some(&body))?;
            Ok(json!({"headers": signed, "params": body, "url": url}))
        }
        Call::Reply {
            query,
            optional_params,
            body,
        } => reply_answer(config, query, optional_params, body),
        Call::Date {
            value,
            dayfirst,
            yearfirst,
        } => date_answer(
            value,
            DateFlags {
                dayfirst: *dayfirst,
                yearfirst: *yearfirst,
            },
        ),
    }
}

// the response decodes with the section filter and count the request stored
fn reply_answer(
    config: &BraveSearchConfig,
    query: &SearchQuery,
    optional_params: &Object,
    body: &str,
) -> Result<Value, BraveSearchAnswerError> {
    let request = config.transform_search_request(query, optional_params)?;
    let _url = config.complete_url(None, optional_params, Some(&request))?;
    let response = config.transform_search_response(&ProviderResponse {
        status_code: 200,
        headers: &Headers::new(),
        text: body,
    })?;
    Ok(serde_json::to_value(response)?)
}

fn date_answer(value: &Value, flags: DateFlags) -> Result<Value, BraveSearchAnswerError> {
    let literal = serde_json::to_string(value)?;
    let today = NaiveDate::from_ymd_opt(2024, 3, 5).unwrap_or_default();
    Ok(json!(to_yyyy_mm_dd(
        &DateValue::from_literal(&literal),
        today,
        flags
    )))
}

fn outcome(result: Result<Value, BraveSearchAnswerError>) -> Value {
    result.map_or_else(
        |_error| json!({"ok": false}),
        |value| json!({"ok": true, "value": value}),
    )
}

mod tests;
