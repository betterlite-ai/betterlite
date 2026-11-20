use std::str::FromStr as _;

use betterlite_logging::fields::payload::records::GuardrailInformation;
use betterlite_logging::prometheus_latency::guardrail_overhead::{
    GuardrailRecords, guardrail_overhead_seconds,
};
use betterlite_logging::prometheus_latency::input_sequence_length::{
    CombinedUsage, InputLengthInputs, UsageMapping, input_sequence_length_bucket,
};
use betterlite_logging::prometheus_latency::latency_histograms::safe_duration_seconds;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Overhead { overhead_calls: Vec<OverheadCall> },
    InputSequenceLength { bucket_calls: Vec<BucketCall> },
    SafeDuration { duration_calls: Vec<DurationCall> },
}

#[derive(Deserialize)]
pub(crate) struct OverheadCall {
    guardrail_information: Value,
}

#[derive(Deserialize)]
pub(crate) struct BucketCall {
    prompt_tokens: Option<i64>,
    combined_usage: Option<CombinedUsageCall>,
    response_has_usage: bool,
    metadata_usage: Option<MetadataUsage>,
    response_is_object: bool,
    response_usage_prompt_tokens: Option<i64>,
}

#[derive(Deserialize)]
struct CombinedUsageCall {
    #[serde(default)]
    upstream_total_tokens: Option<i64>,
}

#[derive(Deserialize)]
struct MetadataUsage {
    #[serde(default)]
    prompt_tokens: Option<i64>,
}

#[derive(Deserialize)]
pub(crate) struct DurationCall {
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    start_time: Option<Decimal>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    end_time: Option<Decimal>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Overhead { overhead_calls } => overhead_values(overhead_calls),
            Input::InputSequenceLength { bucket_calls } => {
                let buckets: Vec<Value> = bucket_calls.iter().map(bucket_value).collect();
                Ok(json!({ "buckets": buckets }))
            }
            Input::SafeDuration { duration_calls } => {
                let seconds: Vec<Value> = duration_calls
                    .iter()
                    .map(|call| {
                        seconds_value(safe_duration_seconds(call.start_time, call.end_time))
                    })
                    .collect();
                Ok(json!({ "seconds": seconds }))
            }
        }
    }
}

fn overhead_values(calls: &[OverheadCall]) -> HelperResult {
    let mut seconds: Vec<Value> = Vec::with_capacity(calls.len());
    for call in calls {
        let entries = guardrail_records(&call.guardrail_information)?;
        let records = match entries.as_slice() {
            [single] => GuardrailRecords::Single(single),
            list => GuardrailRecords::Multiple(list),
        };
        seconds.push(seconds_value(guardrail_overhead_seconds(records)));
    }
    Ok(json!({ "seconds": seconds }))
}

fn guardrail_records(value: &Value) -> Result<Vec<GuardrailInformation>, Box<HelperError>> {
    match value {
        Value::Null => Ok(Vec::new()),
        Value::Object(_) => Ok(vec![serde_json::from_value(value.clone())?]),
        Value::Array(items) => {
            let records = items
                .iter()
                .cloned()
                .map(serde_json::from_value::<GuardrailInformation>)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(records)
        }
        Value::Bool(_) | Value::Number(_) | Value::String(_) => Err(invalid(
            "guardrail_information must be a record, a list of records or null",
        )),
    }
}

fn bucket_value(call: &BucketCall) -> Value {
    let inputs = InputLengthInputs {
        prompt_tokens: call.prompt_tokens,
        combined_usage: call.combined_usage.as_ref().map(|usage| CombinedUsage {
            upstream_total_tokens: usage.upstream_total_tokens,
        }),
        response_has_usage: call.response_has_usage,
        metadata_usage: match &call.metadata_usage {
            Some(usage) => UsageMapping::Mapping {
                prompt_tokens: usage.prompt_tokens,
            },
            None => UsageMapping::Absent,
        },
        response_object: if call.response_is_object {
            UsageMapping::Mapping {
                prompt_tokens: call.response_usage_prompt_tokens,
            }
        } else {
            UsageMapping::Absent
        },
    };
    Value::String(input_sequence_length_bucket(&inputs).to_owned())
}

fn seconds_value(seconds: Option<Decimal>) -> Value {
    seconds
        .and_then(seconds_number)
        .map_or(Value::Null, Value::Number)
}

fn seconds_number(seconds: Decimal) -> Option<serde_json::Number> {
    let mut text = seconds.normalize().to_string();
    if !text.contains('.') && !text.contains('e') {
        text.push_str(".0");
    }
    serde_json::Number::from_str(&text).ok()
}

fn invalid(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
