use std::collections::BTreeMap;
use std::mem;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::extraction::RequestTools;

type Data = BTreeMap<String, GuardrailValue>;

// LiteLLM's _modify_request_with_permission_errors: forbidden tools and functions leave their lists and a forbidden forced choice becomes none
pub fn rewrite_forbidden_tools(data: &mut Data, extracted: &RequestTools, denied: &[&str]) {
    if denied.is_empty() {
        return;
    }
    tracing::info!("Blocking {} unauthorized tool uses", denied.len());
    drop_denied(data, "tools", extracted.tools.as_deref(), denied);
    drop_denied(data, "functions", extracted.functions.as_deref(), denied);
    force_none(
        data,
        "tool_choice",
        extracted.tool_choice.as_deref(),
        denied,
    );
    force_none(
        data,
        "function_call",
        extracted.function_call.as_deref(),
        denied,
    );
}

fn drop_denied(data: &mut Data, field: &str, listed: Option<&[Option<String>]>, denied: &[&str]) {
    let Some(names) = listed else {
        return;
    };
    let Some(GuardrailValue::Array(items)) = data.get_mut(field) else {
        return;
    };
    let kept = mem::take(items)
        .into_iter()
        .zip(names)
        .filter(|(_item, name)| !name.as_deref().is_some_and(|tool| denied.contains(&tool)))
        .map(|(item, _name)| item)
        .collect();
    *items = kept;
}

fn force_none(data: &mut Data, field: &str, forced: Option<&str>, denied: &[&str]) {
    if forced.is_some_and(|name| denied.contains(&name)) {
        data.insert(field.to_owned(), GuardrailValue::Text("none".to_owned()));
    }
}

mod tests;
