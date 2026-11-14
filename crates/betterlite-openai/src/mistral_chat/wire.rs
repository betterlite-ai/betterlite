use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct MistralTextBlock {
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub text: String,
}

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
    #[must_use]
    pub fn thinking_text(&self, separator: &str) -> String {
        self.thinking
            .iter()
            .filter(|entry| entry.kind == "text")
            .map(|entry| entry.text.as_str())
            .collect::<Vec<_>>()
            .join(separator)
    }
}
