const ACCEPTED_MODELS: [&str; 10] = [
    "gpt",
    "claude",
    "gemini",
    "command-r",
    "command-r-plus",
    "command-light",
    "command-medium",
    "command-medium-beta",
    "command-xlarge-nightly",
    "command-nightly",
];

const FALLBACK_MODEL: &str = "gpt-3.5-turbo";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedModel {
    pub name: String,
    pub vertex_ai: bool,
    pub claude_format: bool,
}

// A name only has to contain an accepted word, and a Vertex AI model keeps its name whatever it is.
#[must_use]
pub fn resolve(model: &str, custom_llm_provider: &str) -> ResolvedModel {
    let vertex_ai = custom_llm_provider == "vertex_ai" || model.starts_with("vertex_ai/");
    let accepted = vertex_ai || ACCEPTED_MODELS.iter().any(|word| model.contains(word));
    let name = if accepted { model } else { FALLBACK_MODEL };
    ResolvedModel {
        name: name.to_owned(),
        vertex_ai,
        claude_format: name.contains("claude") && !vertex_ai,
    }
}

mod tests;
