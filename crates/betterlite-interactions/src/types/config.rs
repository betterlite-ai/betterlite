use serde::{Deserialize, Serialize};

use super::enums::{ThinkingLevel, ThinkingSummaries};
use super::tools::ToolChoice;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct GenerationConfig {
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub seed: Option<i64>,
    pub stop_sequences: Option<Vec<String>>,
    pub tool_choice: Option<ToolChoice>,
    pub thinking_level: Option<ThinkingLevel>,
    pub thinking_summaries: Option<ThinkingSummaries>,
    pub max_output_tokens: Option<i64>,
    pub speech_config: Option<Vec<SpeechConfig>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct SpeechConfig {
    pub voice: Option<String>,
    pub language: Option<String>,
    pub speaker: Option<String>,
}
