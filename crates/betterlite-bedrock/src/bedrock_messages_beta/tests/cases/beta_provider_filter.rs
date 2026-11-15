use betterlite_prompts::arguments::json::Json;

use crate::bedrock_messages_beta::tests::support::{
    Call, Fixture, Session, capture_logs, headers, js, list, obj, params,
};

fn resolve(call: &mut Call) -> Vec<String> {
    let fixture = Fixture::new();
    let mut session = Session::new(&fixture);
    session
        .betas(call.request())
        .expect("the beta headers resolve")
}

fn auto_betas() {
    let (_guard, logs) = capture_logs();
    let mut call = Call {
        model: "claude-sonnet-4-5".to_owned(),
        subclass_provider: Some("anthropic".to_owned()),
        messages: list(
            r#"[{"role": "user", "content": [{"type": "file", "file": {"file_id": "file-abc"}}]}]"#,
        ),
        mcp_servers: list(r#"[{"type": "url", "url": "https://example.test/mcp"}]"#),
        tools: list(
            r#"[{"type": "computer_20250124", "name": "computer"}, {"name": "geo", "input_schema": {}, "eager_input_streaming": true}]"#,
        ),
        injected: true,
        safeguards: Some(js("[]")),
        header_values: headers(r#"{"anthropic-beta": "mcp-client-2025-04-04"}"#),
        ..Call::default()
    };
    assert_eq!(
        resolve(&mut call),
        [
            "code-execution-2025-05-22",
            "computer-use-2025-01-24",
            "dangerous-tool-use-2026-09-03",
            "files-api-2025-04-14",
            "fine-grained-tool-streaming-2025-05-14",
            "interleaved-thinking-2025-05-14",
            "mcp-client-2025-04-04",
        ]
    );
    assert!(logs.warnings().is_empty());
}

// the mid-conversation and display betas read the outgoing request instead
fn outgoing_betas() {
    let mut call = Call {
        messages: list(r#"[{"role": "user", "content": "hi"}]"#),
        subclass_provider: Some("anthropic".to_owned()),
        outgoing: obj(r#"{
  "messages": [
    {"role": "assistant", "output_config": {"format": {"type": "json_schema"}}},
    {"role": "system", "content": [{"type": "tool_addition", "tool": {"type": "tool_reference", "name": "geo"}}]}
  ],
  "thinking": {"type": "adaptive", "display": "updates"}
}"#),
        ..Call::default()
    };
    assert_eq!(
        resolve(&mut call),
        [
            "mid-conversation-output-config-2026-07-01",
            "mid-conversation-tool-changes-2026-07-01",
            "thinking-display-updates-2026-08-18",
        ]
    );
}

// an empty safeguards list counts, a null one does not
fn safeguards_cases() {
    let cases: [(Option<Json>, &[&str]); 3] = [
        (Some(js("[]")), &["dangerous-tool-use-2026-09-03"]),
        (Some(js("null")), &[]),
        (None, &[]),
    ];
    for (safeguards, expected) in cases {
        let mut call = Call {
            model: "fixture-no-flag".to_owned(),
            subclass_provider: Some("bedrock_mantle".to_owned()),
            safeguards,
            ..Call::default()
        };
        assert_eq!(resolve(&mut call), expected.to_vec());
    }
}

// the feature params reach the beta list
fn feature_params() {
    let mut call = Call {
        model: "fixture-no-flag".to_owned(),
        subclass_provider: Some("bedrock_mantle".to_owned()),
        params: params(r#"{"output_config": {"effort": "low"}}"#),
        ..Call::default()
    };
    assert_eq!(resolve(&mut call), ["effort-2025-11-24"]);
}

// the context filter runs inside the composition and rewrites the outgoing request
fn context_rewrite() {
    let mut call = Call {
        model: "fixture-tool-search".to_owned(),
        subclass_provider: Some("bedrock_mantle".to_owned()),
        tools: list(r#"[{"type": "tool_search_tool_regex_20251119", "name": "tool_search"}]"#),
        header_values: headers(r#"{"anthropic-beta": "advanced-tool-use-2025-11-20"}"#),
        outgoing: obj(r#"{
  "messages": [],
  "context_management": {"edits": [{"type": "clear_thinking_20251015"}, {"type": "compact_20260112"}]}
}"#),
        ..Call::default()
    };
    assert_eq!(
        resolve(&mut call),
        [
            "compact-2026-01-12",
            "tool-examples-2025-10-29",
            "tool-search-tool-2025-10-19",
        ]
    );
    assert_eq!(
        call.outgoing,
        obj(r#"{
  "messages": [],
  "context_management": {"edits": [{"type": "compact_20260112"}]}
}"#)
    );
}

pub(crate) fn run() {
    auto_betas();
    outgoing_betas();
    safeguards_cases();
    feature_params();
    context_rewrite();
}
