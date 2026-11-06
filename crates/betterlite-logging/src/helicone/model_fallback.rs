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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    OpenAi,
    Claude,
    VertexAi,
    Gemini,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedModel {
    pub name: String,
    pub provider: Provider,
}

// A name only has to contain an accepted word, and a Vertex AI model keeps its name whatever it is.
#[must_use]
pub fn resolve(model: &str, custom_llm_provider: &str) -> ResolvedModel {
    let vertex_ai = custom_llm_provider == "vertex_ai" || model.starts_with("vertex_ai/");
    let accepted = vertex_ai || ACCEPTED_MODELS.iter().any(|word| model.contains(word));
    let name = if accepted { model } else { FALLBACK_MODEL };
    ResolvedModel {
        name: name.to_owned(),
        provider: classify(name, vertex_ai),
    }
}

// Vertex AI wins over the name-based routes, then Claude, then Gemini, and the rest goes to OpenAI.
fn classify(name: &str, vertex_ai: bool) -> Provider {
    if vertex_ai {
        Provider::VertexAi
    } else if name.contains("claude") {
        Provider::Claude
    } else if name.contains("gemini") {
        Provider::Gemini
    } else {
        Provider::OpenAi
    }
}

mod tests;
