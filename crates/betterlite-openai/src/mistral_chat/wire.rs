use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct MistralTextBlock {
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub text: String,
}

// a native content block: text carries "text", a thinking block carries "thinking"
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct MistralContentBlock {
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub thinking: Vec<MistralTextBlock>,
}

impl MistralContentBlock {
    // LiteLLM joins every text entry of one thinking block without a separator
    #[must_use]
    pub fn thinking_text(&self) -> String {
        self.thinking
            .iter()
            .filter(|entry| entry.kind == "text")
            .map(|entry| entry.text.as_str())
            .collect()
    }
}
