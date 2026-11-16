use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Object as Params;
use betterlite_provider::petals::config::{
    REQUEST_HEADERS, SUPPORTED_OPENAI_PARAMS, map_openai_params,
};
use betterlite_provider::petals::error::get_error_class;
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::helper::HelperResult;

use super::petals_config;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ParamsInput {
    #[serde(default)]
    env: BTreeMap<String, String>,
    cases: Vec<MapCase>,
    validate: Vec<ValidateCase>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MapCase {
    non_default: Params,
    #[serde(default)]
    optional: Params,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ValidateCase {
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    api_key: Option<String>,
}

pub(crate) fn params_value(input: &ParamsInput) -> HelperResult {
    let config = petals_config(&input.env)?;
    let mapped: Vec<Value> = input
        .cases
        .iter()
        .map(|case| {
            let mut options = case.optional.clone();
            map_openai_params(&case.non_default, &mut options);
            json!({"options": options})
        })
        .collect();
    let validated: Vec<Value> = input
        .validate
        .iter()
        .map(|case| {
            json!({
                "headers": REQUEST_HEADERS,
                "given": case.headers,
                "api_key": case.api_key,
            })
        })
        .collect();
    let error = get_error_class("boom", 418, &BTreeMap::new());
    Ok(json!({
        "config": config.get_config(),
        "supported": SUPPORTED_OPENAI_PARAMS,
        "mapped": mapped,
        "validated": validated,
        "error": {"status": error.status_code},
    }))
}
