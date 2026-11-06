use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::{Deserialize, Serialize};

pub type DetectorParams = BTreeMap<String, GuardrailValue>;
pub type ServerDetections = Vec<Vec<Detection>>;

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Detection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detection: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detection_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidences: Option<Vec<GuardrailValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<BTreeMap<String, GuardrailValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detector_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DetectorServerRequest<'request> {
    pub contents: &'request [String],
    pub detector_params: &'request DetectorParams,
}

#[derive(Debug, Serialize)]
pub struct OrchestratorRequest<'request> {
    pub content: &'request str,
    pub detectors: BTreeMap<&'request str, &'request DetectorParams>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct OrchestratorResponse {
    pub detections: Vec<Detection>,
}
