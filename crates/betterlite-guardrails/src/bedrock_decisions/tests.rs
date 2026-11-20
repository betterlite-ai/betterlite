#![cfg(test)]

mod bedrock_decision_behaviors;
mod bedrock_decision_boundaries;

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use serde::Serialize;
use serde_json::value::RawValue;

use crate::bedrock_decisions::logging::CallLog;
use crate::bedrock_decisions::tracing::Pricing;
use crate::bedrock_records::Response;
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use crate::guardrail_records::error::GuardrailSpanError;
use crate::guardrail_records::record::{
    GuardrailInvocationRecord, GuardrailRecordContext, GuardrailSpanSink,
};

#[derive(Serialize)]
struct PriceEntry<'price> {
    litellm_provider: &'static str,
    mode: &'static str,
    guardrail_cost_per_unit: &'price RawValue,
}

struct DiscardSink;

impl GuardrailSpanSink for DiscardSink {
    fn emit(&self, _record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        Ok(())
    }
}

fn response(wire: &str) -> Response {
    serde_json::from_str(wire).expect("the fixture response parses")
}

fn json(wire: &str) -> Json {
    serde_json::from_str(wire).expect("the fixture JSON parses")
}

fn as_json<T>(value: &T) -> Json
where
    T: Serialize,
{
    let text = serde_json::to_string(value).expect("the value serializes");
    serde_json::from_str(&text).expect("the serialized value parses as JSON")
}

fn catalog(prices: &[(&str, &str)]) -> ModelCatalog {
    let mut entries = BTreeMap::new();
    for (name, cost) in prices {
        entries.insert(
            (*name).to_owned(),
            PriceEntry {
                litellm_provider: "bedrock",
                mode: "guardrail",
                guardrail_cost_per_unit: serde_json::from_str(cost).expect("the price map parses"),
            },
        );
    }
    let source = serde_json::to_string(&entries).expect("the fixture catalog serializes");
    ModelCatalog::parse(source.as_bytes(), UnknownFields::Warn).expect("the fixture catalog parses")
}

fn base(name: Option<&str>) -> GuardrailBase {
    GuardrailBase {
        settings: GuardrailSettings {
            guardrail_name: name.map(str::to_owned),
            supported_event_hooks: None,
            event_hook: None,
            default_on: false,
            mask_request_content: false,
            mask_response_content: false,
            violation_message_template: None,
            end_session_after_n_fails: None,
            on_violation: None,
            realtime_violation_message: None,
            on_sensitive_data: None,
            sensitive_data_route_to_model: None,
            sticky_session_routing: true,
            run_in_parallel: false,
            scan_raw_request: false,
            only_scan_new_messages: false,
            timeout: None,
            switches: GuardrailSwitches::default(),
        },
        mode: None,
    }
}

fn empty_request_data() -> BTreeMap<String, GuardrailValue> {
    BTreeMap::new()
}

fn context<'record>(
    base: &'record GuardrailBase,
    request_data: &'record mut BTreeMap<String, GuardrailValue>,
    sink: &'record DiscardSink,
) -> GuardrailRecordContext<'record> {
    GuardrailRecordContext {
        base,
        request_data,
        log_target: None,
        sink,
    }
}

fn call_log(prices: &ModelCatalog) -> CallLog<'_> {
    CallLog {
        guardrail_provider: "bedrock",
        event_type: Some(GuardrailEventHooks::DuringCall),
        start_time: SystemTime::UNIX_EPOCH
            .checked_add(Duration::from_millis(100_250))
            .expect("the test clock is after the epoch"),
        now: SystemTime::UNIX_EPOCH
            .checked_add(Duration::from_millis(102_750))
            .expect("the test clock is after the epoch"),
        pricing: Pricing {
            catalog: prices,
            aws_region_name: None,
        },
    }
}

fn record_json(request_data: &BTreeMap<String, GuardrailValue>) -> Json {
    let Some(GuardrailValue::Object(metadata)) = request_data.get("metadata") else {
        panic!("the record lands in the metadata bucket");
    };
    let Some(GuardrailValue::Array(entries)) =
        metadata.get("standard_logging_guardrail_information")
    else {
        panic!("the records live under standard_logging_guardrail_information");
    };
    assert_eq!(entries.len(), 1);
    as_json(&entries[0])
}
