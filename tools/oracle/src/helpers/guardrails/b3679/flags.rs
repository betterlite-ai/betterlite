use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_translation::StreamingScanKey;
use betterlite_guardrails::unified_dispatch::flags::{is_redundant_scan, resolve_streaming_flag};
use serde::Deserialize;
use serde_json::{Value, json};

use super::support::to_json;
use crate::helpers::HelperResult;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FlagRequest {
    pub(super) attribute: Option<GuardrailValue>,
    pub(super) guardrail_config: Option<BTreeMap<String, GuardrailValue>>,
    #[serde(default)]
    pub(super) optional_params: BTreeMap<String, GuardrailValue>,
    pub(super) name: String,
    pub(super) default: GuardrailValue,
}

pub(super) fn streaming_flag(request: &FlagRequest) -> HelperResult {
    let value = resolve_streaming_flag(
        request.attribute.as_ref(),
        request.guardrail_config.as_ref(),
        &request.optional_params,
        &request.name,
        &request.default,
    );
    Ok(json!({"value": to_json(&value)?}))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScanKeyInput {
    pub(super) texts: Vec<String>,
    #[serde(default)]
    pub(super) tool_calls: Vec<String>,
    #[serde(default)]
    pub(super) stream_ended: bool,
    #[serde(default)]
    pub(super) tool_calls_in_flight: bool,
}

impl ScanKeyInput {
    fn to_key(&self) -> StreamingScanKey {
        StreamingScanKey {
            texts: self.texts.clone(),
            tool_calls: self.tool_calls.clone(),
            stream_ended: self.stream_ended,
            tool_calls_in_flight: self.tool_calls_in_flight,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScanRequest {
    pub(super) scan_key: Option<ScanKeyInput>,
    pub(super) last_scan_key: Option<ScanKeyInput>,
}

pub(super) fn redundant(request: &ScanRequest) -> Value {
    let key = request.scan_key.as_ref().map(ScanKeyInput::to_key);
    let last = request.last_scan_key.as_ref().map(ScanKeyInput::to_key);
    json!({"redundant": is_redundant_scan(key.as_ref(), last.as_ref())})
}
