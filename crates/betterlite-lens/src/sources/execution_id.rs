use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE;

// litellm's opaque ID only matches byte-for-byte with json.dumps' ", " item separator.
#[must_use]
pub fn execution_id(source: &str, team_id: &str, trace_id: &str, trace_ref: &str) -> String {
    let elements = [source, team_id, trace_id, trace_ref]
        .into_iter()
        .map(|value| serde_json::to_string(value).unwrap_or_default())
        .collect::<Vec<String>>()
        .join(", ");
    URL_SAFE.encode(format!("[{elements}]"))
}

mod tests;
