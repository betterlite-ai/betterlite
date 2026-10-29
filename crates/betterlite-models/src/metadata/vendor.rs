use serde::{Deserialize, Serialize};

/// Vertex AI Lyria audio API a model is called through.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VertexAudioApi {
    LyriaPredict,
    LyriaInteractions,
}

/// Whether web search is billed per query or per prompt.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebSearchBillingUnit {
    PerQuery,
    PerPrompt,
}
