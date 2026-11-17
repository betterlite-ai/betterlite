use betterlite_anthropic::headers::AnthropicHeaders;
use betterlite_anthropic::option_primitives::OptionDefaults;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::{Value, json};

use super::support::{
    Fixture, MapFetcher, Mode, RefusingFetcher, Session, body_json, catalog, chat_request,
    headers_of, map_of, messages_of, model_cost_of,
};
use crate::claude_invoke::documents::DocumentError;
use crate::claude_invoke::error::ClaudeInvokeError;
use crate::claude_invoke::request::ClaudeInvokeRequest;
use crate::claude_invoke::tools::ToolSearchToolsError;
use crate::claude_invoke::{STRIPS_BILLING_METADATA, USES_ASYNC_TRANSFORM_REQUEST};
use betterlite_prompts::media::error::MediaError;
use betterlite_prompts::media::limits::MAX_REMOTE_URLS;
use betterlite_prompts::media_fetch::error::ImageFetchError;

fn params(value: &Value) -> IndexMap<String, Json> {
    serde_json::from_value(value.clone()).expect("the case params parse")
}

fn plain_fixture() -> Fixture {
    Fixture::new(&model_cost_of(
        r#"{"bedrock/claude-plain": {"litellm_provider": "bedrock"}}"#,
    ))
    .expect("the fixture builds")
}

fn plain_request<'request>(
    defaults: &'request OptionDefaults,
    catalog: &'request ModelCatalog,
    optional_params: IndexMap<String, Json>,
) -> ClaudeInvokeRequest<'request> {
    chat_request(
        "claude-plain",
        prompt_message(),
        optional_params,
        AnthropicHeaders::default(),
        defaults,
        catalog,
    )
}

fn prompt_message() -> Vec<Json> {
    messages_of(r#"[{"role": "user", "content": "hello"}]"#)
}

fn prepared_message() -> Value {
    json!([{"content": [{"text": "hello", "type": "text"}], "role": "user"}])
}

pub(super) fn bedrock_anthropic_version() {
    let fixture = plain_fixture();
    let mut session = Session::new(&fixture).expect("the session builds");
    let fetcher = MapFetcher::default();
    let defaults = OptionDefaults::default();
    let catalog = catalog();
    let request = plain_request(&defaults, &catalog, map_of(r#"{"max_tokens": 64}"#));
    let prepared = session
        .transform(&request, &fetcher, Mode::Sync)
        .expect("the request transforms");
    assert_eq!(
        body_json(&prepared),
        json!({
            "anthropic_version": "bedrock-2023-05-31",
            "max_tokens": 64,
            "messages": prepared_message(),
        })
    );
    let explicit = plain_request(
        &defaults,
        &catalog,
        map_of(
            r#"{"max_tokens": 64, "anthropic_version": "custom-2024-01-01", "aws_access_key_id": "AKIAEXAMPLE", "aws_secret_access_key": "secret"}"#,
        ),
    );
    let prepared = session
        .transform(&explicit, &fetcher, Mode::Sync)
        .expect("the explicit version transforms");
    assert_eq!(
        body_json(&prepared)["anthropic_version"],
        json!("custom-2024-01-01")
    );
    assert!(body_json(&prepared).get("aws_access_key_id").is_none());
    assert!(body_json(&prepared).get("aws_secret_access_key").is_none());
    let null_version = plain_request(
        &defaults,
        &catalog,
        map_of(r#"{"max_tokens": 64, "anthropic_version": null}"#),
    );
    let prepared = session
        .transform(&null_version, &fetcher, Mode::Sync)
        .expect("the null version transforms");
    assert_eq!(body_json(&prepared)["anthropic_version"], Value::Null);
    assert!(fetcher.calls().is_empty());
}

pub(super) fn adaptive_effort_ceiling() {
    let fixture = Fixture::new(&model_cost_of(
        r#"{
            "bedrock/claude-test": {"litellm_provider": "bedrock", "supports_adaptive_thinking": true, "bedrock_output_config_effort_ceiling": "max"},
            "bedrock/claude-low": {"litellm_provider": "bedrock", "supports_adaptive_thinking": true, "bedrock_output_config_effort_ceiling": "high"},
            "bedrock/claude-free": {"litellm_provider": "bedrock", "supports_adaptive_thinking": true},
            "bedrock/claude-config": {"litellm_provider": "bedrock", "supports_adaptive_thinking": true, "supports_output_config": true, "supports_xhigh_reasoning_effort": true, "bedrock_output_config_effort_ceiling": "max"}
        }"#,
    ))
    .expect("the fixture builds");
    let mut session = Session::new(&fixture).expect("the session builds");
    let cases = [
        ("claude-test", "xhigh", "max"),
        ("claude-low", "xhigh", "high"),
        ("claude-test", "low", "low"),
        ("claude-free", "xhigh", "xhigh"),
    ];
    for (model, requested, mapped) in cases {
        let non_default = params(&json!({"reasoning_effort": requested}));
        let mut optional_params = IndexMap::new();
        session
            .map_params(model, false, &non_default, &mut optional_params)
            .expect("the parameters map");
        assert_eq!(
            serde_json::to_value(&optional_params).expect("the params serialize"),
            json!({
                "output_config": {"effort": mapped},
                "thinking": {"type": "adaptive", "display": "summarized"},
            }),
            "model {model} with effort {requested}"
        );
    }
    let defaults = OptionDefaults::default();
    let catalog = catalog();
    let requested = params(&json!({"max_tokens": 64, "output_config": {"effort": "xhigh"}}));
    let request = chat_request(
        "claude-config",
        prompt_message(),
        requested,
        AnthropicHeaders::default(),
        &defaults,
        &catalog,
    );
    let fetcher = MapFetcher::default();
    let prepared = session
        .transform(&request, &fetcher, Mode::Sync)
        .expect("the output config request transforms");
    assert_eq!(
        body_json(&prepared)["output_config"],
        json!({"effort": "max"})
    );
    assert_eq!(
        serde_json::to_value(&request.optional_params).expect("the request params serialize")["output_config"],
        json!({"effort": "xhigh"})
    );
}

pub(super) fn bedrock_beta_filter() {
    let fixture = plain_fixture();
    let mut session = Session::new(&fixture).expect("the session builds");
    let fetcher = MapFetcher::default();
    let defaults = OptionDefaults::default();
    let catalog = catalog();
    let server = json!([{"type": "url", "url": "https://example.test/mcp", "name": "srv"}]);
    let computer = json!([{
        "type": "computer_20250124",
        "name": "computer",
        "display_width_px": 1024,
        "display_height_px": 768,
    }]);
    let with_user_beta = chat_request(
        "claude-plain",
        prompt_message(),
        params(&json!({"max_tokens": 64, "mcp_servers": server, "tools": computer})),
        headers_of(&[("anthropic-beta", "context-1m-2025-08-07")]),
        &defaults,
        &catalog,
    );
    let prepared = session
        .transform(&with_user_beta, &fetcher, Mode::Sync)
        .expect("the beta request transforms");
    assert_eq!(
        body_json(&prepared)["anthropic_beta"],
        json!(["computer-use-2025-01-24", "context-1m-2025-08-07"])
    );
    let filtered = chat_request(
        "claude-plain",
        prompt_message(),
        params(&json!({"max_tokens": 64, "mcp_servers": server})),
        AnthropicHeaders::default(),
        &defaults,
        &catalog,
    );
    let prepared = session
        .transform(&filtered, &fetcher, Mode::Sync)
        .expect("the filtered request transforms");
    assert!(body_json(&prepared).get("anthropic_beta").is_none());
    let explicit_user = chat_request(
        "claude-plain",
        prompt_message(),
        map_of(r#"{"max_tokens": 64}"#),
        headers_of(&[("anthropic-beta", "mcp-client-2025-11-20")]),
        &defaults,
        &catalog,
    );
    let prepared = session
        .transform(&explicit_user, &fetcher, Mode::Sync)
        .expect("the user beta request transforms");
    assert_eq!(
        body_json(&prepared)["anthropic_beta"],
        json!(["mcp-client-2025-11-20"])
    );
    let conversation = messages_of(
        r#"[{"role": "user", "content": "hello"}, {"role": "assistant", "content": "hi", "output_config": {"format": {"type": "json_schema", "schema": {"type": "object"}}}}]"#,
    );
    let mid_conversation = chat_request(
        "claude-plain",
        conversation,
        map_of(r#"{"max_tokens": 64}"#),
        AnthropicHeaders::default(),
        &defaults,
        &catalog,
    );
    let prepared = session
        .transform(&mid_conversation, &fetcher, Mode::Sync)
        .expect("the mid-conversation request transforms");
    assert_eq!(
        body_json(&prepared)["anthropic_beta"],
        json!(["mid-conversation-output-config-2026-07-01"])
    );
    let display_updates = chat_request(
        "claude-plain",
        prompt_message(),
        params(&json!({"max_tokens": 64, "thinking": {"type": "adaptive", "display": "updates"}})),
        AnthropicHeaders::default(),
        &defaults,
        &catalog,
    );
    let prepared = session
        .transform(&display_updates, &fetcher, Mode::Sync)
        .expect("the display updates request transforms");
    assert_eq!(
        body_json(&prepared)["anthropic_beta"],
        json!(["thinking-display-updates-2026-08-18"])
    );
}

pub(super) fn tool_search_tool_name() {
    let fixture = Fixture::new(&model_cost_of(
        r#"{"bedrock/claude-tools": {"litellm_provider": "bedrock", "supports_tool_search": true}}"#,
    ))
    .expect("the fixture builds");
    let mut session = Session::new(&fixture).expect("the session builds");
    let fetcher = MapFetcher::default();
    let defaults = OptionDefaults::default();
    let catalog = catalog();
    let tools = json!([
        {"type": "tool_search_tool_regex_20251119"},
        {"type": "tool_search_tool_regex_20251119", "name": "custom_search"},
        {"type": "tool_search_tool_regex_20251119", "name": null, "input_schema": {"type": "object", "properties": {}}},
        {"type": "tool_search_tool_bm25_20251119", "name": "search_bm25"},
        {"name": "get_weather", "input_schema": {"type": "object", "properties": {}}, "eager_input_streaming": true, "custom": {"defer_loading": true}},
    ]);
    let request = chat_request(
        "claude-tools",
        prompt_message(),
        params(&json!({"max_tokens": 64, "tools": tools})),
        AnthropicHeaders::default(),
        &defaults,
        &catalog,
    );
    let prepared = session
        .transform(&request, &fetcher, Mode::Sync)
        .expect("the tool search request transforms");
    let body = body_json(&prepared);
    assert_eq!(
        body["tools"],
        json!([
            {"name": "tool_search_tool_regex", "type": "tool_search_tool_regex"},
            {"name": "custom_search", "type": "tool_search_tool_regex"},
            {"name": null, "type": "tool_search_tool_regex", "input_schema": {"type": "object", "properties": {}}},
            {"name": "get_weather", "input_schema": {"type": "object", "properties": {}}, "defer_loading": true},
        ])
    );
    assert_eq!(
        body["anthropic_beta"],
        json!([
            "fine-grained-tool-streaming-2025-05-14",
            "tool-search-tool-2025-10-19"
        ])
    );
    let empty = chat_request(
        "claude-tools",
        prompt_message(),
        params(&json!({"max_tokens": 64, "tools": []})),
        AnthropicHeaders::default(),
        &defaults,
        &catalog,
    );
    let prepared = session
        .transform(&empty, &fetcher, Mode::Sync)
        .expect("the empty tools request transforms");
    assert_eq!(body_json(&prepared)["tools"], json!([]));
    let malformed = chat_request(
        "claude-tools",
        prompt_message(),
        params(&json!({"max_tokens": 64, "tools": [42]})),
        AnthropicHeaders::default(),
        &defaults,
        &catalog,
    );
    match session.transform(&malformed, &fetcher, Mode::Sync) {
        Ok(_prepared) => panic!("the malformed tools entry must fail the request"),
        Err(ClaudeInvokeError::Tools(ToolSearchToolsError::NotObject { index })) => {
            assert_eq!(index, 0);
        }
        Err(other) => panic!("unexpected error: {other}"),
    }
}

pub(super) fn inline_document() {
    let fixture = plain_fixture();
    let mut session = Session::new(&fixture).expect("the session builds");
    let defaults = OptionDefaults::default();
    let catalog = catalog();
    let document = messages_of(
        r#"[{"role": "user", "content": [
            {"type": "text", "text": "read this"},
            {"type": "document", "source": {"type": "url", "url": "https://example.test/doc.pdf"}}
        ]}]"#,
    );
    let request = chat_request(
        "claude-plain",
        document,
        map_of(r#"{"max_tokens": 64}"#),
        AnthropicHeaders::default(),
        &defaults,
        &catalog,
    );
    let fetcher = MapFetcher::new(&[(
        "https://example.test/doc.pdf",
        "data:application/octet-stream;base64,JVBERi0xLjQK",
    )]);
    let prepared = session
        .transform(&request, &fetcher, Mode::Sync)
        .expect("the document request transforms");
    assert_eq!(
        body_json(&prepared)["messages"],
        json!([{
            "content": [
                {"text": "read this", "type": "text"},
                {"source": {"data": "JVBERi0xLjQK", "media_type": "application/pdf", "type": "base64"}, "type": "document"},
            ],
            "role": "user",
        }])
    );
    assert_eq!(fetcher.calls(), vec!["https://example.test/doc.pdf"]);
    let suffixed = messages_of(
        r#"[{"role": "user", "content": [
            {"type": "document", "source": {"type": "url", "url": "https://example.test/doc.pdf?token=1"}}
        ]}]"#,
    );
    let request = chat_request(
        "claude-plain",
        suffixed,
        map_of(r#"{"max_tokens": 64}"#),
        AnthropicHeaders::default(),
        &defaults,
        &catalog,
    );
    let streamed = MapFetcher::new(&[(
        "https://example.test/doc.pdf?token=1",
        "data:application/octet-stream;base64,JVBERi0xLjQK",
    )]);
    let prepared = session
        .transform(&request, &streamed, Mode::Sync)
        .expect("the suffixed document transforms");
    assert_eq!(
        body_json(&prepared)["messages"][0]["content"][0]["source"],
        json!({
            "data": "JVBERi0xLjQK",
            "media_type": "application/octet-stream",
            "type": "base64",
        })
    );
    assert_eq!(
        streamed.calls(),
        vec!["https://example.test/doc.pdf?token=1"]
    );
    let inline = messages_of(
        r#"[{"role": "user", "content": [
            {"type": "document", "source": {"type": "base64", "media_type": "application/pdf", "data": "QUJD"}}
        ]}]"#,
    );
    let request = chat_request(
        "claude-plain",
        inline,
        map_of(r#"{"max_tokens": 64}"#),
        AnthropicHeaders::default(),
        &defaults,
        &catalog,
    );
    let quiet = MapFetcher::default();
    let prepared = session
        .transform(&request, &quiet, Mode::Sync)
        .expect("the inline document transforms");
    assert_eq!(
        body_json(&prepared)["messages"][0]["content"][0]["source"]["data"],
        json!("QUJD")
    );
    assert!(quiet.calls().is_empty());
}

pub(super) fn claude_response_usage() {
    let fixture = plain_fixture();
    let mut session = Session::new(&fixture).expect("the session builds");
    let raw = json!({
        "id": "msg_bdrk_01",
        "model": "claude-plain",
        "type": "message",
        "role": "assistant",
        "content": [{"type": "text", "text": "hi there"}],
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 10,
            "output_tokens": 5,
            "cache_creation_input_tokens": 2,
            "cache_read_input_tokens": 3,
        },
    })
    .to_string();
    let envelope = session.response(&raw, prompt_message());
    let usage = envelope.usage.expect("the response carries usage");
    assert_eq!(envelope.id, "generated");
    assert_eq!(envelope.model.as_deref(), Some("claude-plain"));
    let choice = envelope.choices.first().expect("the response has a choice");
    assert_eq!(choice.finish_reason.as_deref(), Some("stop"));
    assert_eq!(choice.message.content.as_deref(), Some("hi there"));
    assert_eq!(usage.prompt_tokens, 15);
    assert_eq!(usage.completion_tokens, 5);
    assert_eq!(usage.total_tokens, 20);
    assert_eq!(usage.cache_creation_input_tokens, 2);
    assert_eq!(usage.cache_read_input_tokens, 3);
    assert_eq!(usage.completion_tokens_details.reasoning, Some(0));
    assert_eq!(usage.completion_tokens_details.text, Some(5));
}

pub(super) fn response_format_tool_stub() {
    let fixture = Fixture::new(&model_cost_of(
        r#"{
            "bedrock/claude-test": {"litellm_provider": "bedrock", "supports_native_structured_output": false, "supports_adaptive_thinking": true},
            "bedrock/claude-native": {"litellm_provider": "bedrock", "supports_native_structured_output": true}
        }"#,
    ))
    .expect("the fixture builds");
    let mut session = Session::new(&fixture).expect("the session builds");
    let schema = json!({
        "type": "json_schema",
        "json_schema": {
            "name": "answer",
            "schema": {
                "type": "object",
                "properties": {"a": {"type": "string"}},
                "required": ["a"],
            },
        },
    });
    let structured = params(&json!({"response_format": schema}));
    let mut optional_params = IndexMap::new();
    session
        .map_params("claude-test", false, &structured, &mut optional_params)
        .expect("the response format maps");
    assert_eq!(
        serde_json::to_value(&optional_params).expect("the params serialize"),
        json!({
            "json_mode": true,
            "tool_choice": {"type": "tool", "name": "json_tool_call"},
            "tools": [{
                "name": "json_tool_call",
                "input_schema": {
                    "type": "object",
                    "properties": {"a": {"type": "string"}},
                    "required": ["a"],
                },
            }],
        })
    );
    let mut native_params = IndexMap::new();
    session
        .map_params("claude-native", false, &structured, &mut native_params)
        .expect("the native response format maps");
    assert_eq!(
        serde_json::to_value(&native_params).expect("the params serialize"),
        json!({
            "json_mode": true,
            "output_format": {
                "type": "json_schema",
                "schema": {
                    "additionalProperties": false,
                    "type": "object",
                    "properties": {"a": {"type": "string"}},
                    "required": ["a"],
                },
            },
        })
    );
    let requested = params(
        &json!({"thinking": {"type": "enabled", "display": "updates", "budget_tokens": 1024}}),
    );
    let mut updates = requested.clone();
    session
        .map_params("claude-test", false, &requested, &mut updates)
        .expect("the display update maps");
    assert_eq!(
        serde_json::to_value(&updates).expect("the params serialize")["thinking"],
        json!({"type": "adaptive", "display": "updates"})
    );
    assert_eq!(
        session.supported("claude-test"),
        vec![
            "stream",
            "stop",
            "temperature",
            "top_p",
            "max_tokens",
            "max_completion_tokens",
            "tools",
            "tool_choice",
            "extra_headers",
            "parallel_tool_calls",
            "response_format",
            "user",
            "web_search_options",
            "speed",
            "context_management",
            "cache_control",
            "thinking",
            "reasoning_effort",
        ]
    );
}

pub(super) fn forced_tool_choice_removed() {
    let fixture = Fixture::new(&model_cost_of(
        r#"{
            "bedrock/claude-test": {"litellm_provider": "bedrock", "supports_forced_tool_use": false},
            "bedrock/claude-native": {"litellm_provider": "bedrock", "supports_forced_tool_use": true}
        }"#,
    ))
    .expect("the fixture builds");
    let mut session = Session::new(&fixture).expect("the session builds");
    let schema = json!({
        "type": "json_schema",
        "json_schema": {"name": "answer", "schema": {"type": "object", "properties": {}}},
    });
    let structured = params(&json!({"response_format": schema}));
    let choice = json!({"type": "function", "function": {"name": "json_tool_call"}});
    let mut removed = params(&json!({"tool_choice": choice}));
    session
        .map_params("claude-test", false, &structured, &mut removed)
        .expect("the unsupported model maps");
    assert_eq!(
        serde_json::to_value(&removed).expect("the params serialize"),
        json!({
            "json_mode": true,
            "tools": [{"name": "json_tool_call", "input_schema": {"type": "object", "properties": {}}}],
        })
    );
    let mut kept = params(&json!({"tool_choice": choice}));
    session
        .map_params("claude-native", false, &structured, &mut kept)
        .expect("the supported model maps");
    assert_eq!(
        serde_json::to_value(&kept).expect("the params serialize"),
        json!({
            "json_mode": true,
            "tools": [{"name": "json_tool_call", "input_schema": {"type": "object", "properties": {}}}],
            "tool_choice": {"type": "tool", "name": "json_tool_call"},
        })
    );
    let other_name = json!({"type": "function", "function": {"name": "get_weather"}});
    let mut kept_other = params(&json!({"tool_choice": other_name}));
    session
        .map_params("claude-test", false, &structured, &mut kept_other)
        .expect("the other tool name maps");
    assert_eq!(
        serde_json::to_value(&kept_other).expect("the params serialize")["tool_choice"],
        json!({"type": "tool", "name": "get_weather"})
    );
    let mut kept_auto = params(&json!({"tool_choice": "auto"}));
    session
        .map_params("claude-test", false, &structured, &mut kept_auto)
        .expect("the auto choice maps");
    assert_eq!(
        serde_json::to_value(&kept_auto).expect("the params serialize")["tool_choice"],
        json!({"type": "auto"})
    );
}

pub(super) fn remote_url_inlined() {
    let fixture = plain_fixture();
    let mut session = Session::new(&fixture).expect("the session builds");
    let defaults = OptionDefaults::default();
    let catalog = catalog();
    let image = messages_of(
        r#"[{"role": "user", "content": [
            {"type": "image_url", "image_url": {"url": "https://example.test/pic.png"}}
        ]}]"#,
    );
    let request = chat_request(
        "claude-plain",
        image,
        map_of(r#"{"max_tokens": 64}"#),
        AnthropicHeaders::default(),
        &defaults,
        &catalog,
    );
    let fetcher = MapFetcher::new(&[(
        "https://example.test/pic.png",
        "data:image/png;base64,iVBORw0KGgo=",
    )]);
    let prepared = session
        .transform(&request, &fetcher, Mode::Async)
        .expect("the image request transforms");
    assert_eq!(
        body_json(&prepared)["messages"],
        json!([{
            "content": [{
                "source": {"data": "iVBORw0KGgo=", "media_type": "image/png", "type": "base64"},
                "type": "image",
            }],
            "role": "user",
        }])
    );
    assert_eq!(fetcher.calls(), vec!["https://example.test/pic.png"]);
}

pub(super) fn async_media_order() {
    assert_eq!([USES_ASYNC_TRANSFORM_REQUEST], [true]);
    let fixture = plain_fixture();
    let mut session = Session::new(&fixture).expect("the session builds");
    let defaults = OptionDefaults::default();
    let catalog = catalog();
    let mixed = messages_of(
        r#"[{"role": "user", "content": [
            {"type": "document", "source": {"type": "url", "url": "https://example.test/second.pdf"}},
            {"type": "image_url", "image_url": "https://example.test/pic.png"},
            {"type": "text", "text": "and this"}
        ]}]"#,
    );
    let request = chat_request(
        "claude-plain",
        mixed,
        map_of(r#"{"max_tokens": 64}"#),
        AnthropicHeaders::default(),
        &defaults,
        &catalog,
    );
    let fetcher = MapFetcher::new(&[
        (
            "https://example.test/pic.png",
            "data:image/png;base64,iVBORw0KGgo=",
        ),
        (
            "https://example.test/second.pdf",
            "data:application/pdf;base64,U0VDT05E",
        ),
    ]);
    let prepared = session
        .transform(&request, &fetcher, Mode::Async)
        .expect("the mixed request transforms");
    assert_eq!(
        body_json(&prepared)["messages"],
        json!([{
            "content": [
                {"source": {"data": "U0VDT05E", "media_type": "application/pdf", "type": "base64"}, "type": "document"},
                {"source": {"data": "iVBORw0KGgo=", "media_type": "image/png", "type": "base64"}, "type": "image"},
                {"text": "and this", "type": "text"},
            ],
            "role": "user",
        }])
    );
    assert_eq!(
        fetcher.calls(),
        vec![
            "https://example.test/second.pdf",
            "https://example.test/pic.png",
        ]
    );
}

pub(super) fn media_fetch() {
    let fixture = plain_fixture();
    let mut session = Session::new(&fixture).expect("the session builds");
    let defaults = OptionDefaults::default();
    let catalog = catalog();
    let mixed = messages_of(
        r#"[{"role": "user", "content": [
            {"type": "image_url", "image_url": "https://example.test/pic.png"},
            {"type": "document", "source": {"type": "url", "url": "https://example.test/doc.pdf"}}
        ]}]"#,
    );
    let fetching = MapFetcher::new(&[
        (
            "https://example.test/pic.png",
            "data:image/png;base64,iVBORw0KGgo=",
        ),
        (
            "https://example.test/doc.pdf",
            "data:application/octet-stream;base64,JVBERi0xLjQK",
        ),
    ]);
    let request = chat_request(
        "claude-plain",
        mixed.clone(),
        map_of(r#"{"max_tokens": 64}"#),
        AnthropicHeaders::default(),
        &defaults,
        &catalog,
    );
    let prepared = session
        .transform(&request, &fetching, Mode::Async)
        .expect("the media request transforms");
    let body = body_json(&prepared);
    assert_eq!(
        body["messages"][0]["content"][0]["source"],
        json!({"data": "iVBORw0KGgo=", "media_type": "image/png", "type": "base64"})
    );
    assert_eq!(
        body["messages"][0]["content"][1]["source"],
        json!({"data": "JVBERi0xLjQK", "media_type": "application/pdf", "type": "base64"})
    );
    assert_eq!(fetching.transport_attempts(), 2);
    let refusing = RefusingFetcher::default();
    let request = chat_request(
        "claude-plain",
        mixed,
        map_of(r#"{"max_tokens": 64}"#),
        AnthropicHeaders::default(),
        &defaults,
        &catalog,
    );
    match session.transform(&request, &refusing, Mode::Async) {
        Ok(_prepared) => panic!("the refusing fetcher must fail the request"),
        Err(ClaudeInvokeError::Media(MediaError::Fetch(ImageFetchError::Rejected { url }))) => {
            assert_eq!(url, "https://example.test/pic.png");
        }
        Err(other) => panic!("unexpected error: {other}"),
    }
    assert_eq!(refusing.transport_attempts(), 0);
    assert_eq!(refusing.calls(), vec!["https://example.test/pic.png"]);
}

pub(super) fn structured_output_body() {
    let fixture = Fixture::new(&model_cost_of(
        r#"{
            "bedrock/claude-native": {"litellm_provider": "bedrock", "supports_native_structured_output": true},
            "bedrock/claude-plain": {"litellm_provider": "bedrock"}
        }"#,
    ))
    .expect("the fixture builds");
    let mut session = Session::new(&fixture).expect("the session builds");
    let defaults = OptionDefaults::default();
    let catalog = catalog();
    let fetcher = MapFetcher::default();
    let schema =
        json!({"type": "object", "properties": {"a": {"type": "string"}}, "required": ["a"]});
    let format = json!({"type": "json_schema", "schema": schema});
    let native = chat_request(
        "claude-native",
        prompt_message(),
        params(&json!({"max_tokens": 64, "output_format": format})),
        AnthropicHeaders::default(),
        &defaults,
        &catalog,
    );
    let prepared = session
        .transform(&native, &fetcher, Mode::Sync)
        .expect("the native structured output transforms");
    let body = body_json(&prepared);
    assert_eq!(body["output_config"], json!({"format": format}));
    assert!(body.get("output_format").is_none());
    let unsupported = chat_request(
        "claude-plain",
        prompt_message(),
        params(&json!({"max_tokens": 64, "output_config": {"format": format, "stream": true}})),
        AnthropicHeaders::default(),
        &defaults,
        &catalog,
    );
    let prepared = session
        .transform(&unsupported, &fetcher, Mode::Sync)
        .expect("the unsupported structured output transforms");
    let body = body_json(&prepared);
    assert!(body.get("output_config").is_none());
    assert_eq!(
        body["messages"][0]["content"][0],
        json!({"text": "hello", "type": "text"})
    );
    let inlined = body["messages"][0]["content"][1]["text"]
        .as_str()
        .expect("the schema is inlined as text");
    assert_eq!(
        serde_json::from_str::<Value>(inlined).expect("the inlined schema parses"),
        schema
    );
}

fn document_bodies(count: usize) -> Vec<Json> {
    let mut messages = Vec::with_capacity(count);
    for index in 0..count {
        let message = json!({
            "role": "user",
            "content": [{
                "type": "document",
                "source": {"type": "url", "url": format!("https://example.test/{index}.pdf")},
            }],
        });
        messages.push(serde_json::from_value(message).expect("the generated message parses"));
    }
    messages
}

fn repeated_document_bodies(count: usize) -> Vec<Json> {
    let blocks: Vec<Value> = (0..count)
        .map(|_| {
            json!({
                "type": "document",
                "source": {"type": "url", "url": "https://example.test/same.pdf"},
            })
        })
        .collect();
    let message = json!({"role": "user", "content": blocks});
    vec![serde_json::from_value(message).expect("the repeated message parses")]
}

pub(super) fn remote_url_limit() {
    let fixture = plain_fixture();
    let mut session = Session::new(&fixture).expect("the session builds");
    let defaults = OptionDefaults::default();
    let catalog = catalog();
    let fetcher = MapFetcher::default();
    let over = chat_request(
        "claude-plain",
        document_bodies(MAX_REMOTE_URLS + 1),
        map_of(r#"{"max_tokens": 64}"#),
        AnthropicHeaders::default(),
        &defaults,
        &catalog,
    );
    match session.transform(&over, &fetcher, Mode::Sync) {
        Ok(_prepared) => panic!("more than {MAX_REMOTE_URLS} document URLs must fail the request"),
        Err(ClaudeInvokeError::Documents(DocumentError::Media(
            MediaError::TooManyRemoteUrls { limit },
        ))) => {
            assert_eq!(limit, MAX_REMOTE_URLS);
        }
        Err(other) => panic!("unexpected error: {other}"),
    }
    assert!(fetcher.calls().is_empty());
    let within = chat_request(
        "claude-plain",
        document_bodies(MAX_REMOTE_URLS),
        map_of(r#"{"max_tokens": 64}"#),
        AnthropicHeaders::default(),
        &defaults,
        &catalog,
    );
    match session.transform(&within, &fetcher, Mode::Sync) {
        Ok(_prepared) => panic!("the empty fetcher cannot serve the document URLs"),
        Err(ClaudeInvokeError::Documents(DocumentError::Fetch(ImageFetchError::Attempts {
            ..
        }))) => {}
        Err(other) => panic!("unexpected error: {other}"),
    }
    let quiet = MapFetcher::default();
    let repeated_over = chat_request(
        "claude-plain",
        repeated_document_bodies(MAX_REMOTE_URLS + 1),
        map_of(r#"{"max_tokens": 64}"#),
        AnthropicHeaders::default(),
        &defaults,
        &catalog,
    );
    match session.transform(&repeated_over, &quiet, Mode::Sync) {
        Ok(_prepared) => {
            panic!("one URL repeated more than {MAX_REMOTE_URLS} times must fail the request")
        }
        Err(ClaudeInvokeError::Documents(DocumentError::Media(
            MediaError::TooManyRemoteUrls { limit },
        ))) => {
            assert_eq!(limit, MAX_REMOTE_URLS);
        }
        Err(other) => panic!("unexpected error: {other}"),
    }
    assert!(quiet.calls().is_empty());
    let served = MapFetcher::new(&[(
        "https://example.test/same.pdf",
        "data:application/pdf;base64,U0VN",
    )]);
    let repeated = chat_request(
        "claude-plain",
        repeated_document_bodies(MAX_REMOTE_URLS),
        map_of(r#"{"max_tokens": 64}"#),
        AnthropicHeaders::default(),
        &defaults,
        &catalog,
    );
    session
        .transform(&repeated, &served, Mode::Sync)
        .expect("one URL repeated up to the ceiling transforms");
    assert_eq!(served.calls().len(), MAX_REMOTE_URLS);
}

pub(super) fn billing_header() {
    let fixture = plain_fixture();
    let mut session = Session::new(&fixture).expect("the session builds");
    let defaults = OptionDefaults::default();
    let catalog = catalog();
    let billing = messages_of(
        r#"[{"role": "system", "content": "x-anthropic-billing-header: abc"}, {"role": "user", "content": "hello"}]"#,
    );
    let request = chat_request(
        "claude-plain",
        billing,
        map_of(r#"{"max_tokens": 64}"#),
        AnthropicHeaders::default(),
        &defaults,
        &catalog,
    );
    let fetcher = MapFetcher::default();
    let prepared = session
        .transform(&request, &fetcher, Mode::Sync)
        .expect("the billing request transforms");
    assert!(body_json(&prepared).get("system").is_none());
    assert_eq!([STRIPS_BILLING_METADATA], [true]);
}
