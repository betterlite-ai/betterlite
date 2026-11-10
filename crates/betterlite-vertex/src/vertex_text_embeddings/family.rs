#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Numeric,
    Bge,
    Standard,
}

// LiteLLM tests isdigit() before BGE detection, so a numeric endpoint id never takes the prompt path
#[must_use]
pub fn family_of(model: &str) -> Family {
    if is_numeric_model(model) {
        return Family::Numeric;
    }
    if is_bge_model(model) {
        return Family::Bge;
    }
    Family::Standard
}

fn is_numeric_model(model: &str) -> bool {
    if model.is_empty() {
        return false;
    }
    model.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_bge_model(model: &str) -> bool {
    let lowered = model.to_lowercase();
    if lowered.starts_with("bge/") {
        return true;
    }
    lowered.contains("bge")
}
