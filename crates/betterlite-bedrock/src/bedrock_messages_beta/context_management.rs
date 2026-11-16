use std::collections::BTreeSet;

use betterlite_prompts::arguments::json::{Json, Object};

const SUPPORTED_EDITS: [(&str, &str); 2] = [
    ("compact_20260112", "compact-2026-01-12"),
    ("clear_tool_uses_20250919", "context-management-2025-06-27"),
];

// clear_thinking_20251015 is absent on purpose: B621 consumes it before this filter runs.
pub fn filter_context_management(request: &mut Object, betas: &mut BTreeSet<String>) {
    let Some(Json::Object(management)) = request.get("context_management") else {
        return;
    };
    let mut updated = management.clone();
    let Some(Json::List(edits)) = updated.get("edits") else {
        request.shift_remove("context_management");
        return;
    };
    let mut retained = Vec::new();
    for edit in edits {
        if let Some(beta) = edit_beta(edit) {
            betas.insert(beta.to_owned());
            retained.push(edit.clone());
        }
    }
    if retained.is_empty() {
        request.shift_remove("context_management");
        return;
    }
    updated.insert("edits".to_owned(), Json::List(retained));
    request.insert("context_management".to_owned(), Json::Object(updated));
}

fn edit_beta(edit: &Json) -> Option<&'static str> {
    let kind = edit.as_object()?.get("type")?.as_str()?;
    for (name, beta) in SUPPORTED_EDITS {
        if name == kind {
            return Some(beta);
        }
    }
    None
}
