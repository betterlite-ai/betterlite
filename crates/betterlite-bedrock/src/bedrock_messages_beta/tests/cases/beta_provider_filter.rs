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

pub(crate) fn run() {
    // auto betas read the original tools and messages, and the extras add their own
    let mut call = Call::default();
    call.model = "claude-sonnet-4-5".to_owned();
    call.subclass_provider = Some("anthropic".to_owned());
    call.messages = list(
        r#"[{"role": "user", "content": [{"type": "file", "file": {"file_id": "file-abc"}}]}]"#,
    );
    call.mcp_servers = list(r#"[{"type": "url", "url": "https://example.test/mcp"}]"#);
    call.tools = list(
        r#"[{"type": "computer_20250124", "name": "computer"}, {"name": "geo", "input_schema": {}, "eager_input_streaming": true}]"#,
    );
    call.injected = true;
    call.safeguards = Some(js("[]"));
    call.header_values = headers(r#"{"anthropic-beta": "mcp-client-2025-04-04"}"#);
    let (_guard, logs) = capture_logs();
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

    // the mid-conversation and display betas read the outgoing request instead
    let mut call = Call::default();
    call.messages = list(r#"[{"role": "user", "content": "hi"}]"#);
    call.subclass_provider = Some("anthropic".to_owned());
    call.outgoing = obj(r#"{
  "messages": [
    {"role": "assistant", "output_config": {"format": {"type": "json_schema"}}},
    {"role": "system", "content": [{"type": "tool_addition", "tool": {"type": "tool_reference", "name": "geo"}}]}
  ],
  "thinking": {"type": "adaptive", "display": "updates"}
}"#);
    assert_eq!(
        resolve(&mut call),
        [
            "mid-conversation-output-config-2026-07-01",
            "mid-conversation-tool-changes-2026-07-01",
            "thinking-display-updates-2026-08-18",
        ]
    );

    // an empty safeguards list counts, a null one does not
    let safeguards_cases: [(Option<betterlite_prompts::arguments::json::Json>, &[&str]); 3] = [
        (Some(js("[]")), &["dangerous-tool-use-2026-09-03"]),
        (Some(js("null")), &[]),
        (None, &[]),
    ];
    for (safeguards, expected) in safeguards_cases {
        let mut call = Call::default();
        call.model = "fixture-no-flag".to_owned();
        call.subclass_provider = Some("bedrock_mantle".to_owned());
        call.safeguards = safeguards;
        assert_eq!(resolve(&mut call), expected.to_vec());
    }

    // the feature params reach the beta list
    let mut call = Call::default();
    call.model = "fixture-no-flag".to_owned();
    call.subclass_provider = Some("bedrock_mantle".to_owned());
    call.params = params(r#"{"output_config": {"effort": "low"}}"#);
    assert_eq!(resolve(&mut call), ["effort-2025-11-24"]);

    // the context filter runs inside the composition and rewrites the outgoing request
    let mut call = Call::default();
    call.model = "fixture-tool-search".to_owned();
    call.subclass_provider = Some("bedrock_mantle".to_owned());
    call.tools = list(r#"[{"type": "tool_search_tool_regex_20251119", "name": "tool_search"}]"#);
    call.header_values = headers(r#"{"anthropic-beta": "advanced-tool-use-2025-11-20"}"#);
    call.outgoing = obj(r#"{
  "messages": [],
  "context_management": {"edits": [{"type": "clear_thinking_20251015"}, {"type": "compact_20260112"}]}
}"#);
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
