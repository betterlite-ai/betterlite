#[must_use]
pub fn sanitize_label_name(label: &str) -> String {
    if label.is_empty() {
        return "_".to_owned();
    }
    let mut sanitized = label
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    if sanitized
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_digit())
    {
        sanitized.insert(0, '_');
    }
    sanitized
}

#[must_use]
pub fn sanitize_label_value(input: Option<&str>) -> Option<String> {
    let text = input?;
    let translated = text
        .chars()
        .filter_map(|character| match character {
            '\n' => Some(' '),
            '\r' | '\u{2028}' | '\u{2029}' => None,
            other => Some(other),
        })
        .collect::<String>();
    let mut escaped = String::with_capacity(translated.len());
    for character in translated.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            other => escaped.push(other),
        }
    }
    Some(escaped)
}
