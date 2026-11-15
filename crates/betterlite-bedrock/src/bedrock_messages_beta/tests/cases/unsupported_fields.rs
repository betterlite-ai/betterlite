use crate::bedrock_messages_beta::fields::strip_unsupported_fields;
use crate::bedrock_messages_beta::tests::support::{capture_logs, js, obj};

const ALLOWED: &str = r#"{
  "output_config": {"format": {"type": "json_schema"}},
  "stop_sequences": ["END"],
  "tools": [{"name": "geo", "input_schema": {}}],
  "messages": [{"role": "user", "content": "hi"}],
  "top_k": 3,
  "thinking": {"type": "enabled", "budget_tokens": 16},
  "temperature": 0.2,
  "tool_choice": {"type": "auto"},
  "anthropic_beta": ["effort-2025-11-24"],
  "metadata": {"user_id": "u"},
  "top_p": 0.9,
  "system": "be brief",
  "safeguards": [],
  "max_tokens": 8,
  "context_management": {"edits": []},
  "anthropic_version": "bedrock-2023-05-31"
}"#;

const ALLOWED_ORDER: [&str; 16] = [
    "output_config",
    "stop_sequences",
    "tools",
    "messages",
    "top_k",
    "thinking",
    "temperature",
    "tool_choice",
    "anthropic_beta",
    "metadata",
    "top_p",
    "system",
    "safeguards",
    "max_tokens",
    "context_management",
    "anthropic_version",
];

pub(crate) fn run() {
    let (_guard, logs) = capture_logs();

    let request = obj(
        r#"{"stream": true, "output_config": {"format": {}}, "max_tokens": 8, "model": "claude", "metadata": {"user_id": "u"}}"#,
    );
    let mut kept = strip_unsupported_fields(&request).expect("the request strips");
    assert_eq!(
        kept,
        obj(r#"{"output_config": {"format": {}}, "max_tokens": 8, "metadata": {"user_id": "u"}}"#)
    );
    assert_eq!(
        logs.debugs(),
        ["Bedrock Invoke: stripping unsupported top-level request fields: ['model', 'stream']"]
    );
    assert!(logs.warnings().is_empty());

    // the caller's mapping survives: the result is a fresh one
    kept.insert("added".to_owned(), js("1"));
    assert!(!request.contains_key("added"));
    assert!(request.contains_key("stream"));

    // every allowed field survives in its input order, output_config included
    let full = obj(ALLOWED);
    let all_kept = strip_unsupported_fields(&full).expect("the allowed request strips");
    let order: Vec<&str> = all_kept.keys().map(String::as_str).collect();
    assert_eq!(order, ALLOWED_ORDER);
    assert_eq!(all_kept, full);

    // nothing to strip leaves the logs silent
    assert_eq!(logs.debugs().len(), 1);
    let (_fresh_guard, fresh_logs) = capture_logs();
    assert!(
        strip_unsupported_fields(&obj(r#"{"max_tokens": 8}"#))
            .expect("an allowed request strips")
            .contains_key("max_tokens")
    );
    assert!(fresh_logs.debugs().is_empty());
    assert!(fresh_logs.warnings().is_empty());

    // an empty request has nothing to keep
    assert!(
        strip_unsupported_fields(&obj("{}"))
            .expect("the empty request strips")
            .is_empty()
    );
}
