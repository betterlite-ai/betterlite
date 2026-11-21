use betterlite_openai::openai_base_config::Object;

use crate::rejected_fields::{EXTRA_INPUTS_MARKER, PASS_THROUGH_HINT};

pub(super) const CHAT: &str = r#"{"model":"deployment","messages":[{"role":"user","content":"hi"}],"temperature":2,"top_p":0.5}"#;
pub(super) const CHAT_WITHOUT_TEMPERATURE: &str =
    r#"{"model":"deployment","messages":[{"role":"user","content":"hi"}],"top_p":0.5}"#;
pub(super) const TOOLS: &str = r#"{"model":"deployment","tools":[{"type":"function","extra":1},{"type":"function","extra":5}]}"#;
pub(super) const TOOLS_WITHOUT_EXTRA: &str =
    r#"{"model":"deployment","tools":[{"type":"function"},{"type":"function"}]}"#;

#[must_use]
pub(super) fn body(source: &str) -> Object {
    serde_json::from_str(source).expect("the fixture body")
}

// Foundry's response text: a JSON error object whose message is a string, as httpx reads it
#[must_use]
pub(super) fn wrapped(message: &str) -> String {
    serde_json::json!({
        "error": {"message": message, "type": "invalid_request_error"},
    })
    .to_string()
}

// the pydantic 422 whose message names one rejected top-level field
#[must_use]
pub(super) fn extra_inputs(field: &str) -> String {
    let message = serde_json::json!({
        "detail": [{"loc": ["body", field], "type": "extra_forbidden"}],
        "msg": EXTRA_INPUTS_MARKER,
    })
    .to_string();
    wrapped(&message)
}

// the pass-through hint in a message that is not JSON, so the final loc drop reads no detail
#[must_use]
pub(super) fn pass_through(message: &str) -> String {
    wrapped(&format!("{message} {PASS_THROUGH_HINT}"))
}
