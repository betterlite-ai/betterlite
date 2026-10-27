use super::lunary::{clean_extra_params, parse_messages, parse_tool_calls, parse_usage};
use super::wandb::OpenAIRequestResponseResolver;
use serde_json::json;

#[test]
fn parses_usage_with_zero_fallback() {
    assert_eq!(
        parse_usage(&json!({"prompt_tokens": 4})),
        json!({"completion": 0, "prompt": 4})
    );
}

#[test]
fn parses_messages_and_tool_call_variants() {
    let message = json!({"role":"assistant", "content":"done", "tool_calls":[
        {"type":"function", "id":"call-1", "function":{"name":"search","arguments":"{}"}},
        {"type":"custom", "id":"call-2", "custom":{"name":"lookup","input":{"q":"x"}}},
        {"type":"function", "id":"ignored"}
    ]});
    assert_eq!(
        parse_messages(&json!([message])),
        Some(json!({
            "role":"assistant", "content":"done", "tool_calls":[
                {"type":"function", "id":"call-1", "function":{"name":"search", "arguments":"{}"}},
                {"type":"custom", "id":"call-2", "function":{"name":"lookup", "arguments":{"q":"x"}}}
            ]
        }))
    );
    assert_eq!(parse_tool_calls(&json!(null)), json!(null));
}

#[test]
fn cleans_extra_params_without_stringifying_tools() {
    let (params, template) = clean_extra_params(&json!({
        "extra_body": {"ignored": true}, "user": "ignored",
        "extra_headers": {"Template-Id": "template-1", "Other": "ignored"},
        "tools": [{"type": "function"}], "metadata": {"nested": true}
    }));
    assert_eq!(template, Some("template-1".to_owned()));
    assert_eq!(
        params,
        json!({"tools": [{"type": "function"}], "metadata": "{\"nested\":true}"})
    );
}

#[test]
fn resolves_completion_trace_tree() {
    let request = json!({"prompt":"hello"});
    let response = json!({"object":"text_completion", "model":"gpt", "created":10,
        "choices":[{"text":"world"}]});
    let trace = OpenAIRequestResponseResolver::resolve(&request, &response, 0.25).expect("trace");
    assert_eq!(
        trace.span.results[0].outputs["response"],
        "\n\n**Completion**: world\n"
    );
    assert_eq!(trace.span.start_time_ms, 10_000);
    assert_eq!(trace.span.end_time_ms, 10_250);
}
