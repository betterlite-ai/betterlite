use super::route::VertexModelRoute;

#[must_use]
pub fn get_vertex_base_model_name(model: &str) -> &str {
    for route in VertexModelRoute::ALL {
        let Some(stripped) = model.strip_prefix(route.prefix()) else {
            continue;
        };
        return stripped;
    }
    model
}

#[must_use]
pub fn get_fine_tuned_endpoint_id(model: &str) -> Option<&str> {
    let candidate = if model.contains("gemini/") {
        model.rsplit('/').next().unwrap_or(model)
    } else {
        model
    };
    is_decimal(candidate).then_some(candidate)
}

#[must_use]
pub fn get_model_for_vertex_ai_url(model: &str) -> &str {
    if model.contains("gemini/") {
        return model.rsplit('/').next().unwrap_or(model);
    }
    model
}

#[must_use]
pub fn is_gemini_3_or_newer(model: &str) -> bool {
    let name = model.rsplit('/').next().unwrap_or(model).to_lowercase();
    if name.is_empty() || name.starts_with("gemma-") || is_fine_tuned(model, &name) {
        return false;
    }
    !is_pre_gemini_3_name(&name)
}

#[must_use]
pub(crate) fn is_decimal(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    text.bytes().all(|byte| byte.is_ascii_digit())
}

// The "gemini/" spec prefix on an unprefixed name marks a deployment, not a Gemini model id
fn is_fine_tuned(model: &str, name: &str) -> bool {
    if model.starts_with("gemini/") && !name.starts_with("gemini-") {
        return true;
    }
    is_decimal(name)
}

fn is_pre_gemini_3_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("gemini-") else {
        return false;
    };
    if let Some(tail) = version_tail(rest) {
        return tail.is_empty() || tail.starts_with('-');
    }
    if let Some(tail) = rest.strip_prefix("exp") {
        return tail.is_empty() || tail.starts_with('-');
    }
    for family in ["pro", "flash"] {
        let Some(tail) = rest.strip_prefix(family) else {
            continue;
        };
        return (tail.is_empty() || tail.starts_with('-'))
            && !matches!(tail, "-latest" | "-lite-latest");
    }
    false
}

fn version_tail(rest: &str) -> Option<&str> {
    let after_digit = rest.strip_prefix(['1', '2'])?;
    let Some(fraction) = after_digit.strip_prefix('.') else {
        return Some(after_digit);
    };
    let digits = fraction
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(fraction.len());
    if digits == 0 {
        return None;
    }
    fraction.get(digits..)
}
