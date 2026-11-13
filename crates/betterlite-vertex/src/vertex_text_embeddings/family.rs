#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Numeric,
    Bge,
    Standard,
}

// LiteLLM tests isdigit() before BGE detection, so a numeric endpoint id never takes the prompt path
#[must_use]
pub fn family_of(model: &str) -> Family {
    if !model.is_empty() && model.bytes().all(|byte| byte.is_ascii_digit()) {
        return Family::Numeric;
    }
    if model.to_lowercase().contains("bge") {
        return Family::Bge;
    }
    Family::Standard
}
