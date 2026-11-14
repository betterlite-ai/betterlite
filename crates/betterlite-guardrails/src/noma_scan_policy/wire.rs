use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use super::error::NomaError;

// LiteLLM's scan answer; the provider may add fields, which ride along in `rest`
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct ScanResponse {
    // an absent key defaults to False (safe), while an explicit null is a failed response
    #[serde(
        default,
        rename = "aggregatedScanResult",
        deserialize_with = "serde_with::rust::double_option::deserialize",
        skip_serializing_if = "Option::is_none"
    )]
    pub aggregated_scan_result: Option<Option<GuardrailValue>>,
    // a missing key defaults to an empty scan list, while an explicit null stays null in the error detail
    #[serde(
        default,
        rename = "scanResult",
        deserialize_with = "serde_with::rust::double_option::deserialize",
        skip_serializing_if = "Option::is_none"
    )]
    pub scan_result: Option<Option<GuardrailValue>>,
    #[serde(flatten)]
    pub rest: BTreeMap<String, GuardrailValue>,
}

impl ScanResponse {
    // Parses through a JSON value first, so a repeated key keeps its last value as in Python
    pub fn from_body(body: &[u8]) -> Result<Self, NomaError> {
        let parsed =
            serde_json::from_slice(body).map_err(|source| NomaError::ResponseJson { source })?;
        serde_json::from_value(parsed).map_err(|source| NomaError::ResponseShape { source })
    }
}

#[derive(Debug, Serialize)]
pub(super) struct ScanPayload<'input> {
    pub(super) input: &'input [Box<RawValue>],
    #[serde(rename = "x-noma-context")]
    pub(super) context: NomaContext,
}

// The body field LiteLLM builds from request metadata, user auth and the guardrail defaults
#[derive(Debug, Serialize)]
pub(super) struct NomaContext {
    #[serde(rename = "applicationId")]
    pub(super) application_id: GuardrailValue,
    #[serde(rename = "ipAddress")]
    pub(super) ip_address: GuardrailValue,
    #[serde(rename = "userId")]
    pub(super) user_id: GuardrailValue,
    #[serde(rename = "sessionId")]
    pub(super) session_id: GuardrailValue,
    #[serde(rename = "requestId")]
    pub(super) request_id: GuardrailValue,
}

#[derive(Debug, Serialize)]
pub(super) struct InputMessage<'text> {
    #[serde(rename = "type")]
    pub(super) kind: &'static str,
    pub(super) role: &'static str,
    pub(super) content: [TextPart<'text>; 1],
}

#[derive(Debug, Serialize)]
pub(super) struct TextPart<'text> {
    #[serde(rename = "type")]
    pub(super) kind: &'static str,
    pub(super) text: &'text str,
}
