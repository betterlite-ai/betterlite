use betterlite_prompts::arguments::json::Json;

use crate::responses::request::transform_inlined_request;

use super::support::{MODEL, capture, input, object};

const CODEX_INPUT: &str = r#"[
    {"type": "agent_message", "content": [{"type": "output_text", "text": "hello"}]},
    {"type": "context_compaction", "encrypted_content": "enc"},
    {"type": "message", "role": "user", "content": "keep"}
]"#;

const NORMALIZED: &str = r#"[
    {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "hello"}]},
    {"type": "compaction", "encrypted_content": "enc"},
    {"type": "message", "role": "user", "content": "keep"}
]"#;

const REWRITE_WARNING: &str = "Bedrock Runtime Responses API: rewrote Codex input item type(s) ('agent_message', 'context_compaction') that the endpoint rejects.";

pub(super) fn codex_input_rewrite() {
    let (messages, _guard) = capture();
    let params = object(r#"{"temperature": 0.5}"#);
    let doubled = format!("chat_completions/chat_completions/{MODEL}");
    let body = transform_inlined_request(&doubled, &input(CODEX_INPUT), &params, None)
        .expect("the request transforms");
    let expected: Json = serde_json::from_str(NORMALIZED).expect("fixture items");
    assert_eq!(body.get("input"), Some(&expected));
    assert_eq!(
        body.get("model").and_then(Json::as_str),
        Some(format!("chat_completions/{MODEL}").as_str())
    );
    assert_eq!(body.get("temperature"), params.get("temperature"));
    assert_eq!(messages.warnings(), vec![REWRITE_WARNING.to_owned()]);
    let single =
        r#"[{"type": "agent_message", "content": [{"type": "output_text", "text": "x"}]}]"#;
    transform_inlined_request(MODEL, &input(single), &params, None)
        .expect("one rewrite transforms");
    assert_eq!(
        messages.warnings().last().map(String::as_str),
        Some(
            "Bedrock Runtime Responses API: rewrote Codex input item type(s) ('agent_message',) that the endpoint rejects."
        )
    );
    let plain = input(r#"[{"type": "message", "role": "user", "content": "keep"}]"#);
    let routed =
        transform_inlined_request(&format!("chat_completions/{MODEL}"), &plain, &params, None)
            .expect("a plain request transforms");
    assert_eq!(routed.get("model").and_then(Json::as_str), Some(MODEL));
    let text = transform_inlined_request(MODEL, &input(r#""hello""#), &params, None)
        .expect("text transforms");
    assert_eq!(text.get("input").and_then(Json::as_str), Some("hello"));
    assert_eq!(messages.warnings().len(), 2);
}
