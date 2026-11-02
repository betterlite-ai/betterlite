use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

pub type JavelinExtraValue = GuardrailValue;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JavelinGuardInput {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JavelinGuardRequest {
    pub input: JavelinGuardInput,
    pub config: BTreeMap<String, JavelinExtraValue>,
    pub metadata: BTreeMap<String, JavelinExtraValue>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JavelinGuardResponse {
    #[serde(default)]
    pub assessments: Vec<IndexMap<String, JavelinAssessment>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JavelinAssessment {
    #[serde(default)]
    pub request_reject: bool,
    #[serde(default)]
    pub results: JavelinAssessmentResults,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JavelinAssessmentResults {
    #[serde(default)]
    pub reject_prompt: Option<String>,
}

#[must_use]
pub fn find_rejection(response: &JavelinGuardResponse) -> Option<&JavelinAssessment> {
    // LiteLLM nests a loop over assessments and their items, breaking at the first request_reject; IndexMap keeps insertion order so this lazy chain stops at the same element.
    response
        .assessments
        .iter()
        .flat_map(IndexMap::values)
        .find(|assessment| assessment.request_reject)
}
