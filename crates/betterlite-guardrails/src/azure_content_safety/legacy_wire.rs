use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
pub enum TextCategory {
    #[serde(rename = "Hate")]
    Hate,
    #[serde(rename = "SelfHarm")]
    SelfHarm,
    #[serde(rename = "Sexual")]
    Sexual,
    #[serde(rename = "Violence")]
    Violence,
}

impl TextCategory {
    pub const ALL: [Self; 4] = [Self::Hate, Self::SelfHarm, Self::Sexual, Self::Violence];

    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        if name == "Hate" {
            return Some(Self::Hate);
        }
        if name == "SelfHarm" {
            return Some(Self::SelfHarm);
        }
        if name == "Sexual" {
            return Some(Self::Sexual);
        }
        if name == "Violence" {
            return Some(Self::Violence);
        }
        None
    }
}

#[derive(Debug, Serialize)]
pub struct AnalyzeTextRequest<'text> {
    pub text: &'text str,
    #[serde(rename = "outputType")]
    pub output_type: AnalyzeTextOutputType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum AnalyzeTextOutputType {
    #[serde(rename = "FourSeverityLevels")]
    FourSeverityLevels,
    #[serde(rename = "EightSeverityLevels")]
    EightSeverityLevels,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AnalyzeTextResponse {
    #[serde(rename = "categoriesAnalysis")]
    pub categories_analysis: Vec<CategoryAnalysis>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CategoryAnalysis {
    pub category: TextCategory,
    #[serde(default)]
    pub severity: Option<i64>,
}

// The ModelResponse fields the legacy post-call hook reads; the wire-format crate owns the full response.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ModelResponse {
    pub choices: Vec<ResponseChoice>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ResponseChoice {
    pub message: Option<ResponseMessage>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ResponseMessage {
    pub content: Option<GuardrailValue>,
}
