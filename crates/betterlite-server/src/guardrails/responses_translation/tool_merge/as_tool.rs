use betterlite_core::api::responses_tool_namespaces::values::{Fields, member_list};
use betterlite_models::decisions::Json;

mod tests;

// A guardrail can return a value of any JSON type; only an object is a tool.
#[must_use]
pub const fn as_tool(value: &Json) -> Option<&Fields> {
    match value {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

// The log line is the only effect, and a test cannot see it.
#[cfg_attr(test, mutants::skip)]
fn warn_when_dropped(dropped: usize) {
    if dropped > 0 {
        tracing::warn!("Dropping {dropped} guardrail-returned tools that are not objects");
    }
}

#[must_use]
pub fn validated_tools(values: &[Json]) -> Vec<&Fields> {
    let validated: Vec<&Fields> = values.iter().filter_map(as_tool).collect();
    warn_when_dropped(values.len().saturating_sub(validated.len()));
    validated
}

// A namespace without a tools list has no members, and a member that is not an object is not kept.
#[must_use]
pub fn namespace_members(namespace: &Fields) -> Vec<&Fields> {
    member_list(namespace)
        .unwrap_or_default()
        .iter()
        .filter_map(as_tool)
        .collect()
}
