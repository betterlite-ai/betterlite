use serde::{Deserialize, Serialize};

use crate::azure_content_safety::legacy_wire::TextCategory;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BodyOptionalParams {
    pub categories: Vec<TextCategory>,
    #[serde(rename = "blocklistNames")]
    pub blocklist_names: Option<Vec<String>>,
    #[serde(rename = "haltOnBlocklistHit")]
    pub halt_on_blocklist_hit: bool,
    #[serde(rename = "outputType")]
    pub output_type: OutputType,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
pub enum OutputType {
    #[default]
    #[serde(rename = "FourSeverityLevels")]
    FourSeverityLevels,
    #[serde(rename = "EightSeverityLevels")]
    EightSeverityLevels,
}

#[derive(Debug, Serialize)]
pub struct AnalyzeRequest<'text> {
    pub text: &'text str,
    pub categories: &'text [TextCategory],
    #[serde(rename = "blocklistNames")]
    pub blocklist_names: Option<&'text [String]>,
    #[serde(rename = "haltOnBlocklistHit")]
    pub halt_on_blocklist_hit: bool,
    #[serde(rename = "outputType")]
    pub output_type: OutputType,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AnalyzeResponse {
    #[serde(rename = "categoriesAnalysis")]
    pub categories_analysis: Vec<CategoryAnalysis>,
}

// LiteLLM declares the category as plain text here, so a name outside the four is kept as it arrives.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CategoryAnalysis {
    pub category: String,
    pub severity: i64,
}
