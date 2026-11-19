mod fakes;
mod output;

use std::collections::BTreeMap;

use betterlite_rate_limit::limiter_records::{
    CacheCounterValue, RateLimitDescriptor, WindowKeyMetadata,
};
use betterlite_rate_limit::windowed_admission::WindowedAdmission;
use betterlite_rate_limit::windowed_admission::over_limit::is_cache_list_over_limit;
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use fakes::admission;
use output::{over_limit_error, response_json};

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum CounterValueInput {
    Int(i64),
    Float(f64),
    Text(String),
}

fn counter_value(value: Option<&CounterValueInput>) -> Option<CacheCounterValue> {
    value.map(|value| match value {
        CounterValueInput::Int(number) => CacheCounterValue::Int(*number),
        CounterValueInput::Float(number) => CacheCounterValue::Float(*number),
        CounterValueInput::Text(text) => CacheCounterValue::Text(text.clone()),
    })
}

#[derive(Debug, Deserialize)]
pub(crate) struct CollectInput {
    descriptors: Vec<RateLimitDescriptor>,
    skip_tpm_check: bool,
    window_size: i64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OverLimitInput {
    keys_to_fetch: Vec<String>,
    cache_values: Vec<Option<CounterValueInput>>,
    key_metadata: BTreeMap<String, WindowKeyMetadata>,
    read_only: bool,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Collect(CollectInput),
    CheckOverLimit(OverLimitInput),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Collect(call) => Ok(collect(call)),
            Input::CheckOverLimit(call) => check_over_limit(call),
        }
    }
}

fn collect(call: &CollectInput) -> Value {
    let limiter: WindowedAdmission = admission(call.window_size);
    let collected =
        limiter.collect_windowed_keys_and_gauges(&call.descriptors, call.skip_tpm_check);
    json!({
        "keys_to_fetch": collected.keys_to_fetch,
        "key_metadata": collected.key_metadata,
        "gauges": collected.gauges,
    })
}

fn check_over_limit(call: &OverLimitInput) -> HelperResult {
    let values: Vec<Option<CacheCounterValue>> = call
        .cache_values
        .iter()
        .map(Option::as_ref)
        .map(counter_value)
        .collect();
    let response = is_cache_list_over_limit(
        &call.keys_to_fetch,
        &values,
        &call.key_metadata,
        call.read_only,
    )
    .map_err(|error| over_limit_error(&error))?;
    Ok(json!({ "response": response_json(&response) }))
}
