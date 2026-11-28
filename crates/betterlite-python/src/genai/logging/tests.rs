#![cfg(test)]

use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_sdk::call_wrapper::ports::IdSource;
use betterlite_vertex::genai::session::Entry;
use betterlite_vertex::genai::session::call::GenaiCall;

use super::logging_state;

// one fixed id for every call, so a state minted from the session shows where its ids came from
struct FixedIds;

impl IdSource for FixedIds {
    fn next_call_id(&self) -> String {
        "fixed-id".to_owned()
    }
}

#[test]
fn logging_state_takes_its_ids_from_the_session() {
    let call: GenaiCall =
        serde_json::from_str(r#"{"model": "gemini/gemini-2.5-flash", "contents": []}"#).unwrap();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let state = logging_state(&call, Entry::Blocking, &redactor, &FixedIds).unwrap();
    assert_eq!(state.litellm_call_id, "fixed-id");
    assert_eq!(state.function_id, "fixed-id");
    assert_eq!(state.litellm_trace_id, "fixed-id");
}
