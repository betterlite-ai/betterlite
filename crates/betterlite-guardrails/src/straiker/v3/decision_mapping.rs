use betterlite_prompts::arguments::json::{Json, Object};

// Straiker answers in two envelopes: a relayed body gets the hook contract, `hookSpecificOutput.permissionDecision`, with the flat fields nested under `straiker`; a flat call answers `action` at the top level. Reading only one would make block mode a no-op on the other.
#[must_use]
pub fn decision(body: &Object) -> (Option<String>, &Object) {
    let verdict = body
        .get("straiker")
        .and_then(Json::as_object)
        .unwrap_or(body);
    let permission = body
        .get("hookSpecificOutput")
        .and_then(Json::as_object)
        .and_then(|hook| hook.get("permissionDecision"))
        .and_then(Json::as_str)
        .filter(|text| !text.is_empty());
    let stated = permission.or_else(|| {
        verdict
            .get("action")
            .and_then(Json::as_str)
            .filter(|text| !text.is_empty())
    });
    (stated.map(str::to_lowercase), verdict)
}

mod tests;
