use aws_lc_rs::digest::{SHA256, digest};
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
    let text = format!("{position}|{name}|{arguments}");
    let mut digest = hex::encode(digest(&SHA256, text.as_bytes()));
    digest.truncate(24);
    format!("call_{digest}")
}
