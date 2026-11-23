use betterlite_models::controls::json_value::JsonValue;

use crate::api::responses_create::call::ResponsesCreateCall;

use super::support::{self, Outcome, ScriptedHandoff, ScriptedResolver};

const CALL: &str = r#"{"model":"openai/gpt-4o","input":"hello","text_format":{"type":"json_object"},"instructions":"short","temperature":0,"parallel_tool_calls":false,"store":false,"background":false,"service_tier":"flex","safety_identifier":"user-a","extra_headers":{"X-Trace":"a"}}"#;

pub(super) async fn parameter_handoff() {
    let call = support::call_from(CALL);
    let handoff = ScriptedHandoff::new(Outcome::Completed(Some(support::stamped("resp_a"))));
    let resolver = ScriptedResolver::default();
    support::drive(&resolver, None, &handoff, &call)
        .await
        .expect("the wrapper answers");
    let calls = handoff.calls();
    assert_eq!(calls.len(), 1);
    supplied(&calls[0]);
    omitted(&calls[0]);
    assert_eq!(handoff.contexts(), vec![String::from("ctx")]);
}

// Every named argument of the call reaches the sync entry unchanged
fn supplied(call: &ResponsesCreateCall) {
    assert_eq!(call.model, "openai/gpt-4o");
    assert_eq!(call.instructions.as_deref(), Some("short"));
    assert_eq!(call.temperature, Some(0.0_f64));
    assert_eq!(call.parallel_tool_calls, Some(false));
    assert_eq!(call.store, Some(false));
    assert_eq!(call.background, Some(false));
    assert_eq!(call.service_tier.as_deref(), Some("flex"));
    assert_eq!(call.safety_identifier.as_deref(), Some("user-a"));
    assert_eq!(
        call.extra_headers
            .as_ref()
            .and_then(|headers| headers.get("X-Trace")),
        Some(&String::from("a"))
    );
    assert_eq!(call.custom_llm_provider.as_deref(), Some("openai"));
}

// text_format becomes text, the sync entry gets the trusted async flag, and the options nobody set stay absent
fn omitted(call: &ResponsesCreateCall) {
    assert_eq!(call.text_format, None);
    assert_eq!(
        call.text,
        Some(support::json(r#"{"format":{"type":"json_object"}}"#))
    );
    assert_eq!(call.kwargs.len(), 1);
    assert_eq!(call.kwargs.get("aresponses"), Some(&JsonValue::Bool(true)));
    assert_eq!(support::input_json(call), support::json("\"hello\""));
    assert_eq!(call.include, None);
    assert_eq!(call.max_output_tokens, None);
    assert_eq!(call.prompt, None);
    assert_eq!(call.metadata, None);
    assert_eq!(call.previous_response_id, None);
    assert_eq!(call.reasoning, None);
    assert_eq!(call.stream, None);
    assert_eq!(
        call.text,
        Some(support::json(r#"{"format":{"type":"json_object"}}"#))
    );
    assert_eq!(call.tool_choice, None);
    assert_eq!(call.tools, None);
    assert_eq!(call.top_p, None);
    assert_eq!(call.truncation, None);
    assert_eq!(call.user, None);
    assert_eq!(call.extra_query, None);
    assert_eq!(call.extra_body, None);
    assert_eq!(call.timeout, None);
    assert_eq!(call.allowed_openai_params, None);
    assert_eq!(call.api_key, None);
    assert_eq!(call.api_base, None);
    assert_eq!(call.ssl_verify, None);
}
