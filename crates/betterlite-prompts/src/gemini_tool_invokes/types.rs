use crate::arguments::json::Json;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ThoughtSignatureFields {
    pub thought_signature: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct GeminiFunctionCallParams {
    pub name: Option<String>,
    // A null arguments field takes the same empty default LiteLLM reads for an absent one.
    pub arguments: Option<String>,
    pub provider_specific_fields: Option<ThoughtSignatureFields>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct GeminiTool {
    pub id: Option<String>,
    pub function: Option<GeminiFunctionCallParams>,
    pub provider_specific_fields: Option<ThoughtSignatureFields>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GeminiFunctionCall {
    pub name: String,
    pub args: Json,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GeminiPart {
    pub function_call: GeminiFunctionCall,
    #[serde(rename = "thoughtSignature", skip_serializing_if = "Option::is_none")]
    pub thought_signature: Option<String>,
}
