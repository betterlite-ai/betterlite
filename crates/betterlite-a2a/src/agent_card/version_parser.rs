use betterlite_core::agents::json::AgentJson;

#[must_use]
pub fn normalize_version_bytes(bytes: &[u8]) -> Option<&'static str> {
    let value = serde_json::from_slice::<AgentJson>(bytes).ok()?;
    super::normalize_protocol_version(&value)
}
