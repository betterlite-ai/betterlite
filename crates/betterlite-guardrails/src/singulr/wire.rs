use serde::Deserialize;

// LiteLLM validates the answer with SingulrGuardrailResponse, whose two fields both have a default
#[derive(Debug, Deserialize)]
pub struct Answer {
    #[serde(default)]
    pub should_block: bool,
    #[serde(default)]
    pub blocking_due_to: Option<String>,
}
