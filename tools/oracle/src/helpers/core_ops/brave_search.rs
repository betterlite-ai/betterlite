use std::collections::BTreeMap;

use betterlite_core::api::search_providers::brave::BraveSearchConfig;
use betterlite_core::api::search_providers::brave::dates::{DateFlags, DateValue, to_yyyy_mm_dd};
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
        #[serde(default)]
        api_key: Option<String>,
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
    Date {
        value: Value,
        #[serde(default)]
        dayfirst: bool,
        #[serde(default)]
        yearfirst: bool,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let today = NaiveDate::from_ymd_opt(2024, 3, 5).unwrap_or_default();
        let config = BraveSearchConfig::load(&input.environment, Some(today));
        let outcomes: Vec<Value> = input
            .calls
            .iter()
            .map(|call| outcome(answer(&config, call)))
            .collect();
        serde_json::to_value(outcomes)
            .map(|calls| json!({"calls": calls}))
            .or_value_error()
    }
}

fn answer(config: &BraveSearchConfig, call: &Call) -> Result<Value, Box<dyn std::error::Error>> {
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
            let (signed, _) = config.sign_request(incoming, &outbound)?;
            let url = config.complete_url(api_base.as_deref(), optional_params, Some(&body))?;
            Ok(json!({"headers": signed, "params": body, "url": url}))
        }
        Call::Reply {
            query,
            optional_params,
            body,
        } => {
            // the response decodes with the section filter and count the request stored
            let params = config.transform_search_request(query, optional_params)?;
            let _url = config.complete_url(None, optional_params, Some(&params))?;
            let response = config.transform_search_response(&ProviderResponse {
                status_code: 200,
                headers: &Headers::new(),
                text: body,
            })?;
            Ok(serde_json::to_value(response)?)
        }
        Call::Date {
            value,
            dayfirst,
            yearfirst,
        } => {
            let literal = serde_json::to_string(value)?;
            let flags = DateFlags {
                dayfirst: *dayfirst,
                yearfirst: *yearfirst,
            };
            let today = NaiveDate::from_ymd_opt(2024, 3, 5).unwrap_or_default();
            let date = to_yyyy_mm_dd(&DateValue::from_literal(&literal), today, flags);
            Ok(json!(date))
        }
    }
}

fn outcome(result: Result<Value, Box<dyn std::error::Error>>) -> Value {
    result.map_or_else(
        |_| json!({"ok": false}),
        |value| json!({"ok": true, "value": value}),
    )
}

mod tests;
