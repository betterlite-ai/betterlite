#[must_use]
pub fn float_text(value: f64) -> String {
    let text = value.to_string();
    if text.contains(['.', 'e', 'i', 'N']) {
        text
    } else {
        format!("{text}.0")
    }
}

#[must_use]
pub fn rounded_to_two(value: f64) -> String {
    let formatted = format!("{value:.2}");
    let rounded = formatted.parse::<f64>().unwrap_or(value);
    float_text(rounded)
}
