use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_guardrails::guardrail_metadata::{
    append_guardrail_response, logging_caching_headers, record_scan_id,
};
use betterlite_guardrails::guardrail_records::record::{
    GuardrailInvocationRecord, GuardrailRecordMode, GuardrailStatus, GuardrailTracingDetail,
};
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperResult, OrHelperError as _};

type Fields = BTreeMap<String, GuardrailValue>;

#[derive(Debug, Deserialize)]
pub(crate) struct RecordScanArgs {
    request_data: Fields,
    scan_id: Option<String>,
    guardrail_name: Option<String>,
    provider: String,
    stage: GuardrailEventHooks,
}

#[derive(Debug, Deserialize)]
pub(crate) struct HeadersArgs {
    request_data: Fields,
}

#[derive(Debug, Deserialize)]
struct RecordArgs {
    guardrail_name: Option<String>,
    guardrail_provider: Option<String>,
    guardrail_mode: Option<GuardrailEventHooks>,
    guardrail_response: GuardrailValue,
    guardrail_status: GuardrailStatus,
    start_time: Option<f64>,
    end_time: Option<f64>,
    duration: Option<f64>,
    masked_entity_count: Option<BTreeMap<String, i64>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AppendArgs {
    model_call_details: Option<Fields>,
    record: RecordArgs,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    RecordScanId(RecordScanArgs),
    Headers(HeadersArgs),
    AppendStandardObject(AppendArgs),
}

fn record_scan(args: &RecordScanArgs) -> HelperResult {
    let mut request_data = args.request_data.clone();
    record_scan_id(
        &mut request_data,
        args.scan_id.as_deref(),
        args.guardrail_name.as_deref(),
        &args.provider,
        args.stage,
    );
    let request_json = serde_json::to_value(&request_data).or_value_error()?;
    Ok(json!({ "request_data": request_json }))
}

fn headers(args: &HeadersArgs) -> HelperResult {
    let headers = logging_caching_headers(&args.request_data).or_value_error()?;
    let mut object = serde_json::Map::new();
    for (name, value) in headers {
        object.insert(name, serde_json::Value::String(value));
    }
    Ok(serde_json::Value::Object(object))
}

fn invocation_record(args: &RecordArgs) -> GuardrailInvocationRecord {
    GuardrailInvocationRecord {
        guardrail_name: args.guardrail_name.clone(),
        guardrail_provider: args.guardrail_provider.clone(),
        guardrail_mode: args.guardrail_mode.map(GuardrailRecordMode::Event),
        guardrail_response: args.guardrail_response.clone(),
        guardrail_status: args.guardrail_status,
        start_time: args.start_time,
        end_time: args.end_time,
        duration: args.duration,
        masked_entity_count: args.masked_entity_count.clone(),
        tracing_detail: GuardrailTracingDetail::default(),
    }
}

fn append_standard_object(args: &AppendArgs) -> HelperResult {
    let record = invocation_record(&args.record);
    let mut details = args.model_call_details.clone();
    append_guardrail_response(details.as_mut(), &record).or_value_error()?;
    let details_json = match details {
        Some(fields) => serde_json::to_value(&fields).or_value_error()?,
        None => serde_json::Value::Null,
    };
    Ok(json!({ "model_call_details": details_json }))
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::RecordScanId(args) => record_scan(args),
            Input::Headers(args) => headers(args),
            Input::AppendStandardObject(args) => append_standard_object(args),
        }
    }
}
