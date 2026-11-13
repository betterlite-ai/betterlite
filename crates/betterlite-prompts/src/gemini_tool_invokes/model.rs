const GEMINI_PREFIX: &str = "gemini-";
const PREFIXED_GEMINI: &str = "gemini/";
const GEMMA_PREFIX: &str = "gemma-";

// B1305 owns VertexGeminiConfig._is_gemini_3_or_newer; this is the same predicate until that layer lands.
#[must_use]
pub(crate) fn is_gemini_3_or_newer(model: &str) -> bool {
    let model_name = model.rsplit('/').next().unwrap_or(model).to_lowercase();
    if model_name.is_empty()
        || model_name.starts_with(GEMMA_PREFIX)
        || is_fine_tuned_model(model, &model_name)
    {
        return false;
    }
    !is_pre_gemini_3(&model_name)
}

fn is_fine_tuned_model(model: &str, model_name: &str) -> bool {
    let numeric = !model_name.is_empty()
        && model_name
            .chars()
            .all(|character| character.is_ascii_digit());
    numeric || (model.starts_with(PREFIXED_GEMINI) && !model_name.starts_with(GEMINI_PREFIX))
}

fn is_pre_gemini_3(model_name: &str) -> bool {
    let Some(rest) = model_name.strip_prefix(GEMINI_PREFIX) else {
        return false;
    };
    if let Some(tail) = rest.strip_prefix("exp") {
        return tail.is_empty() || tail.starts_with('-');
    }
    if let Some(tail) = version_tail(rest) {
        return tail.is_empty() || tail.starts_with('-');
    }
    let Some(tail) = rest
        .strip_prefix("pro")
        .or_else(|| rest.strip_prefix("flash"))
    else {
        return false;
    };
    !matches!(tail, "-latest" | "-lite-latest") && (tail.is_empty() || tail.starts_with('-'))
}

fn version_tail(rest: &str) -> Option<&str> {
    if !rest.starts_with(['1', '2']) {
        return None;
    }
    let after_digit = rest.get(1..)?;
    if let Some(fraction) = after_digit.strip_prefix('.') {
        let digits = fraction
            .find(|character: char| !character.is_ascii_digit())
            .unwrap_or(fraction.len());
        if digits > 0 {
            return after_digit.get(digits.saturating_add(1)..);
        }
    }
    Some(after_digit)
}
