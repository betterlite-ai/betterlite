use std::collections::BTreeMap;

mod tests;

#[must_use]
pub fn merge_mcp_headers(
    extra_headers: Option<&BTreeMap<String, String>>,
    static_headers: Option<&BTreeMap<String, String>>,
) -> Option<BTreeMap<String, String>> {
    let mut merged = extra_headers.cloned().unwrap_or_default();
    if let Some(configured) = static_headers {
        merged.extend(configured.clone());
    }
    (!merged.is_empty()).then_some(merged)
}
