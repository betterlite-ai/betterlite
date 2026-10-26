use super::UsageRow;

pub(super) fn json(row: &UsageRow) -> String {
    let pairs = [
        ("team_id", &row.team_id),
        ("team_alias", &row.team_alias),
        ("organization_id", &row.organization_id),
        ("organization_alias", &row.organization_alias),
        ("user_id", &row.user_id),
        ("user_email", &row.user_email),
        ("api_key_alias", &row.api_key_alias),
        ("model", &row.model),
        ("model_group", &row.model_group),
        ("custom_llm_provider", &row.custom_llm_provider),
    ];
    let entries: Vec<String> = pairs
        .iter()
        .filter_map(|(key, value)| {
            value
                .as_deref()
                .map(|text| format!("\"{key}\": \"{}\"", escaped(text)))
        })
        .collect();
    format!("{{{}}}", entries.join(", "))
}

// LiteLLM writes Tags with json.dumps, which escapes everything outside printable ASCII
fn escaped(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut units = [0; 2];
    for character in text.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\u{8}' => output.push_str("\\b"),
            '\u{c}' => output.push_str("\\f"),
            ' '..='~' => output.push(character),
            _ => output.extend(
                character
                    .encode_utf16(&mut units)
                    .iter()
                    .map(|unit| format!("\\u{unit:04x}")),
            ),
        }
    }
    output
}
