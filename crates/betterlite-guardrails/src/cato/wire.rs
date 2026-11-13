use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AnalyzeRequest {
    pub messages: Vec<GuardrailValue>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AnalyzeResponse {
    pub required_action: Option<RequiredAction>,
    pub analysis_result: Option<AnalysisResult>,
    pub redacted_chat: Option<BTreeMap<String, GuardrailValue>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct RequiredAction {
    pub action_type: Option<String>,
    pub detection_message: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AnalysisResult {
    pub policy_drill_down: Option<BTreeMap<String, GuardrailValue>>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RedactedMessage {
    pub content: Option<GuardrailValue>,
}

impl RedactedMessage {
    #[must_use]
    pub fn from_value(value: &GuardrailValue) -> Self {
        let GuardrailValue::Object(object) = value else {
            return Self::default();
        };
        let content = match object.get("content") {
            None | Some(GuardrailValue::Null) => None,
            Some(content) => Some(content.clone()),
        };
        Self { content }
    }

    #[must_use]
    pub fn to_value(&self) -> GuardrailValue {
        let mut object = BTreeMap::new();
        if let Some(content) = &self.content {
            object.insert("content".to_owned(), content.clone());
        }
        GuardrailValue::Object(object)
    }
}
