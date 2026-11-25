use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;

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
    named_provider().await;
    given_text().await;
}

// A caller's own text wins over text_format, while an explicit null text counts as absent and converts the format
async fn given_text() {
    let cases = [
        (
            r#"{"model":"openai/gpt-4o","input":"hello","text":{"format":{"type":"text"}},"text_format":{"type":"json_object"}}"#,
            false,
            r#"{"format":{"type":"text"}}"#,
        ),
        (
            r#"{"model":"openai/gpt-4o","input":"hello","text_format":{"type":"json_object"}}"#,
            true,
            r#"{"format":{"type":"json_object"}}"#,
        ),
    ];
    for (input, nulled, expected) in cases {
        let mut call = support::call_from(input);
        if nulled {
            call.text = Some(Json::Null);
        }
        let handoff = ScriptedHandoff::new(Outcome::Completed(Some(support::stamped("resp_a"))));
        let resolver = ScriptedResolver::default();
        support::drive(&resolver, None, &handoff, &call)
            .await
            .expect("the wrapper answers");
        let calls = handoff.calls();
        assert_eq!(calls[0].text, Some(support::json(expected)), "{input}");
        assert_eq!(calls[0].text_format, None);
    }
}

// A caller-named provider is forwarded as given, and detection never sees the model
async fn named_provider() {
    let call = support::call_from(
        r#"{"model":"gpt-4o","input":"hello","custom_llm_provider":"anthropic"}"#,
    );
    let handoff = ScriptedHandoff::new(Outcome::Completed(Some(support::stamped("resp_a"))));
    let resolver = ScriptedResolver::default();
    support::drive(&resolver, None, &handoff, &call)
        .await
        .expect("the wrapper answers");
    let calls = handoff.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "gpt-4o");
    assert_eq!(calls[0].custom_llm_provider.as_deref(), Some("anthropic"));
    assert!(resolver.requests().is_empty());
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
    converted(call);
    absent(call);
}

fn converted(call: &ResponsesCreateCall) {
    assert_eq!(call.text_format, None);
    assert_eq!(
        call.text,
        Some(support::json(r#"{"format":{"type":"json_object"}}"#))
    );
    assert_eq!(call.kwargs.len(), 1);
    assert_eq!(call.kwargs.get("aresponses"), Some(&JsonValue::Bool(true)));
    assert_eq!(support::input_json(call), support::json("\"hello\""));
}

// Every named option the caller left out stays absent from the handoff
fn absent(call: &ResponsesCreateCall) {
    let fields = [
        ("include", call.include.is_none()),
        ("max_output_tokens", call.max_output_tokens.is_none()),
        ("prompt", call.prompt.is_none()),
        ("metadata", call.metadata.is_none()),
        ("previous_response_id", call.previous_response_id.is_none()),
        ("reasoning", call.reasoning.is_none()),
        ("stream", call.stream.is_none()),
        ("tool_choice", call.tool_choice.is_none()),
        ("tools", call.tools.is_none()),
        ("top_p", call.top_p.is_none()),
        ("truncation", call.truncation.is_none()),
        ("user", call.user.is_none()),
        ("extra_query", call.extra_query.is_none()),
        ("extra_body", call.extra_body.is_none()),
        ("timeout", call.timeout.is_none()),
        (
            "allowed_openai_params",
            call.allowed_openai_params.is_none(),
        ),
        ("api_key", call.api_key.is_none()),
        ("api_base", call.api_base.is_none()),
        ("ssl_verify", call.ssl_verify.is_none()),
    ];
    for (name, is_none) in fields {
        assert!(is_none, "{name} stays absent");
    }
}
