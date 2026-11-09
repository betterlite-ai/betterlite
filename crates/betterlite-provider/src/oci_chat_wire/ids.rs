use sha2::{Digest as _, Sha256};

#[must_use]
pub fn normalize_oci_finish_reason(raw: Option<&str>) -> Option<String> {
    let reason = raw?;
    let normalized = match reason {
        "MAX_TOKENS" => "length",
        "TOOL_CALL" | "TOOL_CALLS" => "tool_calls",
        _ => "stop",
    };
    Some(normalized.to_owned())
}

#[must_use]
pub fn synthesize_oci_tool_call_id(position: usize, name: &str, arguments: &str) -> String {
    let mut digest = hex::encode(Sha256::digest(format!("{position}|{name}|{arguments}")));
    digest.truncate(24);
    format!("call_{digest}")
}
