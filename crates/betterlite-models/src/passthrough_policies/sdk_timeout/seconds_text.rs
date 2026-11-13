pub(super) fn parse_seconds_text(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    // pydantic's float adapter accepts underscores only in text with no surrounding space
    if trimmed.len() != text.len() {
        return trimmed.parse::<f64>().ok();
    }
    let cleaned = strip_underscores(text)?;
    cleaned.parse::<f64>().ok()
}

fn strip_underscores(text: &str) -> Option<String> {
    let mut cleaned = String::with_capacity(text.len());
    let mut underscore = false;
    for character in text.chars() {
        if character == '_' {
            if underscore || cleaned.is_empty() {
                return None;
            }
            underscore = true;
            continue;
        }
        underscore = false;
        cleaned.push(character);
    }
    if underscore {
        return None;
    }
    Some(cleaned)
}
