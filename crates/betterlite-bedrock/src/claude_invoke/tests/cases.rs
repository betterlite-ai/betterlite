use std::convert::Infallible;

use betterlite_anthropic::headers::AnthropicHeaders;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::support::{
    Fixture, MapFetcher, Mode, RefusingFetcher, Session, at, field, headers_of, json, map_of,
    messages_of, model_cost_of, plain_body, prompt_message, response, transformed,
};
use crate::claude_invoke::documents::DocumentError;
use crate::claude_invoke::error::ClaudeInvokeError;
use crate::claude_invoke::tools::ToolSearchToolsError;
use crate::claude_invoke::{STRIPS_BILLING_METADATA, USES_ASYNC_TRANSFORM_REQUEST};
use betterlite_prompts::media::error::MediaError;
use betterlite_prompts::media::limits::MAX_REMOTE_URLS;
use betterlite_prompts::media_fetch::error::ImageFetchError;

const PLAIN_COST: &str = r#"{"bedrock/claude-plain": {"litellm_provider": "bedrock"}}"#;
const ADAPTIVE_COST: &str = r#"{
    "bedrock/claude-test": {"litellm_provider": "bedrock", "supports_adaptive_thinking": true, "bedrock_output_config_effort_ceiling": "max"},
    "bedrock/claude-low": {"litellm_provider": "bedrock", "supports_adaptive_thinking": true, "bedrock_output_config_effort_ceiling": "high"},
    "bedrock/claude-free": {"litellm_provider": "bedrock", "supports_adaptive_thinking": true},
    "bedrock/claude-config": {"litellm_provider": "bedrock", "supports_adaptive_thinking": true, "supports_output_config": true, "supports_xhigh_reasoning_effort": true, "bedrock_output_config_effort_ceiling": "max"}
}"#;
const TOOL_COST: &str =
    r#"{"bedrock/claude-tools": {"litellm_provider": "bedrock", "supports_tool_search": true}}"#;
const TOOL_STUB_COST: &str = r#"{
    "bedrock/claude-test": {"litellm_provider": "bedrock", "supports_native_structured_output": false, "supports_adaptive_thinking": true},
    "bedrock/claude-native": {"litellm_provider": "bedrock", "supports_native_structured_output": true}
}"#;
const FORCED_COST: &str = r#"{
    "bedrock/claude-test": {"litellm_provider": "bedrock", "supports_forced_tool_use": false},
    "bedrock/claude-native": {"litellm_provider": "bedrock", "supports_forced_tool_use": true}
}"#;
const OUTPUT_COST: &str = r#"{
    "bedrock/claude-native": {"litellm_provider": "bedrock", "supports_native_structured_output": true},
    "bedrock/claude-plain": {"litellm_provider": "bedrock"}
}"#;
const RESPONSE_FORMAT: &str = r#"{"type": "json_schema", "json_schema": {"name": "answer", "schema": {"type": "object", "properties": {"a": {"type": "string"}}, "required": ["a"]}}}"#;
const FORCED_FORMAT: &str = r#"{"type": "json_schema", "json_schema": {"name": "answer", "schema": {"type": "object", "properties": {}}}}"#;
const OUTPUT_FORMAT: &str = r#"{"type": "json_schema", "schema": {"type": "object", "properties": {"a": {"type": "string"}}, "required": ["a"]}}"#;

fn plain_fixture() -> Fixture {
    Fixture::new(&model_cost_of(PLAIN_COST)).expect("the fixture builds")
}

fn map_params_of(
    session: &mut Session<'_>,
    model: &str,
    non_default: &IndexMap<String, Json>,
    mut optional_params: IndexMap<String, Json>,
) -> Json {
    session
        .map_params(model, false, non_default, &mut optional_params)
        .expect("the parameters map");
    Json::Object(optional_params)
}

fn reasoning_effort(value: &str) -> IndexMap<String, Json> {
    let mut params = IndexMap::new();
    params.insert("reasoning_effort".to_owned(), Json::Text(value.to_owned()));
    params
}

fn choice_params(choice: &str) -> IndexMap<String, Json> {
    let mut params = IndexMap::new();
    params.insert("tool_choice".to_owned(), json(choice));
    params
}

fn structured_params(format: &str) -> IndexMap<String, Json> {
    let mut params = IndexMap::new();
    params.insert("response_format".to_owned(), json(format));
    params
}

fn beta_body(
    fixture: &Fixture,
    session: &mut Session<'_>,
    messages: Vec<Json>,
    params: IndexMap<String, Json>,
    headers: AnthropicHeaders,
) -> Json {
    let request = fixture.request("claude-plain", messages, params, headers);
    let fetcher = MapFetcher::default();
    transformed(session, &request, &fetcher, Mode::Sync).expect("the beta request transforms")
}

pub(super) fn bedrock_anthropic_version() {
    let fixture = plain_fixture();
    let mut session = Session::new(&fixture).expect("the session builds");
    let fetcher = MapFetcher::default();
    let request = fixture.request(
        "claude-plain",
        prompt_message(),
        map_of(r#"{"max_tokens": 64}"#),
        AnthropicHeaders::default(),
    );
    let body =
        transformed(&mut session, &request, &fetcher, Mode::Sync).expect("the request transforms");
    assert_eq!(
        body,
        json(
            r#"{"anthropic_version": "bedrock-2023-05-31", "max_tokens": 64, "messages": [{"content": [{"text": "hello", "type": "text"}], "role": "user"}]}"#,
        )
    );
    explicit_anthropic_version(&fixture, &mut session);
    let null_version = fixture.request(
        "claude-plain",
        prompt_message(),
        map_of(r#"{"max_tokens": 64, "anthropic_version": null}"#),
        AnthropicHeaders::default(),
    );
    let null_body = transformed(&mut session, &null_version, &fetcher, Mode::Sync)
        .expect("the null version transforms");
    assert_eq!(field(&null_body, "anthropic_version"), Some(&Json::Null));
    assert!(fetcher.calls().is_empty());
}

fn explicit_anthropic_version(fixture: &Fixture, session: &mut Session<'_>) {
    let fetcher = MapFetcher::default();
    let request = fixture.request(
        "claude-plain",
        prompt_message(),
        map_of(
            r#"{"max_tokens": 64, "anthropic_version": "custom-2024-01-01", "aws_access_key_id": "AKIAEXAMPLE", "aws_secret_access_key": "secret"}"#,
        ),
        AnthropicHeaders::default(),
    );
    let body = transformed(session, &request, &fetcher, Mode::Sync)
        .expect("the explicit version transforms");
    assert_eq!(
        field(&body, "anthropic_version"),
        Some(&json(r#""custom-2024-01-01""#))
    );
    assert!(field(&body, "aws_access_key_id").is_none());
    assert!(field(&body, "aws_secret_access_key").is_none());
}

pub(super) fn adaptive_effort_ceiling() {
    let fixture = Fixture::new(&model_cost_of(ADAPTIVE_COST)).expect("the fixture builds");
    let mut session = Session::new(&fixture).expect("the session builds");
    let cases = [
        ("claude-test", "xhigh", "max"),
        ("claude-low", "xhigh", "high"),
        ("claude-test", "low", "low"),
        ("claude-free", "xhigh", "xhigh"),
    ];
    for (model, requested, ceiling) in cases {
        let mapped = map_params_of(
            &mut session,
            model,
            &reasoning_effort(requested),
            IndexMap::new(),
        );
        assert_eq!(
            at(&mapped, &["output_config", "effort"]),
            &Json::Text(ceiling.to_owned()),
            "model {model} with effort {requested}"
        );
        assert_eq!(
            field(&mapped, "thinking"),
            Some(&json(r#"{"type": "adaptive", "display": "summarized"}"#))
        );
    }
    output_config_ceiling(&fixture, &mut session);
}

fn output_config_ceiling(fixture: &Fixture, session: &mut Session<'_>) {
    let fetcher = MapFetcher::default();
    let request = fixture.request(
        "claude-config",
        prompt_message(),
        map_of(r#"{"max_tokens": 64, "output_config": {"effort": "xhigh"}}"#),
        AnthropicHeaders::default(),
    );
    let caller_params = Json::Object(request.optional_params.clone());
    let body = transformed(session, &request, &fetcher, Mode::Sync)
        .expect("the output config request transforms");
    assert_eq!(
        field(&body, "output_config"),
        Some(&json(r#"{"effort": "max"}"#))
    );
    assert_eq!(
        at(&caller_params, &["output_config", "effort"]),
        &Json::Text("xhigh".to_owned())
    );
}

pub(super) fn bedrock_beta_filter() {
    let fixture = plain_fixture();
    let mut session = Session::new(&fixture).expect("the session builds");
    user_and_tool_betas(&fixture, &mut session);
    mcp_beta_filtered(&fixture, &mut session);
    user_beta_passes(&fixture, &mut session);
    mid_conversation_beta(&fixture, &mut session);
    thinking_display_beta(&fixture, &mut session);
}

fn user_and_tool_betas(fixture: &Fixture, session: &mut Session<'_>) {
    let params = map_of(
        r#"{"max_tokens": 64, "mcp_servers": [{"type": "url", "url": "https://example.test/mcp", "name": "srv"}], "tools": [{"type": "computer_20250124", "name": "computer", "display_width_px": 1024, "display_height_px": 768}]}"#,
    );
    let body = beta_body(
        fixture,
        session,
        prompt_message(),
        params,
        headers_of(&[("anthropic-beta", "context-1m-2025-08-07")]),
    );
    assert_eq!(
        field(&body, "anthropic_beta"),
        Some(&json(
            r#"["computer-use-2025-01-24", "context-1m-2025-08-07"]"#,
        ))
    );
}

fn mcp_beta_filtered(fixture: &Fixture, session: &mut Session<'_>) {
    let params = map_of(
        r#"{"max_tokens": 64, "mcp_servers": [{"type": "url", "url": "https://example.test/mcp", "name": "srv"}]}"#,
    );
    let body = beta_body(
        fixture,
        session,
        prompt_message(),
        params,
        AnthropicHeaders::default(),
    );
    assert!(field(&body, "anthropic_beta").is_none());
}

fn user_beta_passes(fixture: &Fixture, session: &mut Session<'_>) {
    let body = beta_body(
        fixture,
        session,
        prompt_message(),
        map_of(r#"{"max_tokens": 64}"#),
        headers_of(&[("anthropic-beta", "mcp-client-2025-11-20")]),
    );
    assert_eq!(
        field(&body, "anthropic_beta"),
        Some(&json(r#"["mcp-client-2025-11-20"]"#))
    );
}

fn mid_conversation_beta(fixture: &Fixture, session: &mut Session<'_>) {
    let messages = messages_of(
        r#"[{"role": "user", "content": "hello"}, {"role": "assistant", "content": "hi", "output_config": {"format": {"type": "json_schema", "schema": {"type": "object"}}}}]"#,
    );
    let body = beta_body(
        fixture,
        session,
        messages,
        map_of(r#"{"max_tokens": 64}"#),
        AnthropicHeaders::default(),
    );
    assert_eq!(
        field(&body, "anthropic_beta"),
        Some(&json(r#"["mid-conversation-output-config-2026-07-01"]"#))
    );
}

fn thinking_display_beta(fixture: &Fixture, session: &mut Session<'_>) {
    let params =
        map_of(r#"{"max_tokens": 64, "thinking": {"type": "adaptive", "display": "updates"}}"#);
    let body = beta_body(
        fixture,
        session,
        prompt_message(),
        params,
        AnthropicHeaders::default(),
    );
    assert_eq!(
        field(&body, "anthropic_beta"),
        Some(&json(r#"["thinking-display-updates-2026-08-18"]"#))
    );
}

pub(super) fn tool_search_tool_name() {
    let fixture = Fixture::new(&model_cost_of(TOOL_COST)).expect("the fixture builds");
    let mut session = Session::new(&fixture).expect("the session builds");
    named_tool_search_tools(&fixture, &mut session);
    empty_tools(&fixture, &mut session);
    malformed_tools(&fixture, &mut session);
}

fn named_tool_search_tools(fixture: &Fixture, session: &mut Session<'_>) {
    let params = map_of(
        r#"{"max_tokens": 64, "tools": [
            {"type": "tool_search_tool_regex_20251119"},
            {"type": "tool_search_tool_regex_20251119", "name": "custom_search"},
            {"type": "tool_search_tool_regex_20251119", "name": null, "input_schema": {"type": "object", "properties": {}}},
            {"type": "tool_search_tool_bm25_20251119", "name": "search_bm25"},
            {"name": "get_weather", "input_schema": {"type": "object", "properties": {}}, "eager_input_streaming": true, "custom": {"defer_loading": true}}
        ]}"#,
    );
    let request = fixture.request(
        "claude-tools",
        prompt_message(),
        params,
        AnthropicHeaders::default(),
    );
    let fetcher = MapFetcher::default();
    let body = transformed(session, &request, &fetcher, Mode::Sync)
        .expect("the tool search request transforms");
    assert_eq!(
        field(&body, "tools"),
        Some(&json(
            r#"[
                {"name": "tool_search_tool_regex", "type": "tool_search_tool_regex"},
                {"name": "custom_search", "type": "tool_search_tool_regex"},
                {"name": null, "type": "tool_search_tool_regex", "input_schema": {"type": "object", "properties": {}}},
                {"name": "get_weather", "input_schema": {"type": "object", "properties": {}}, "defer_loading": true}
            ]"#,
        ))
    );
    assert_eq!(
        field(&body, "anthropic_beta"),
        Some(&json(
            r#"["fine-grained-tool-streaming-2025-05-14", "tool-search-tool-2025-10-19"]"#,
        ))
    );
}

fn empty_tools(fixture: &Fixture, session: &mut Session<'_>) {
    let request = fixture.request(
        "claude-tools",
        prompt_message(),
        map_of(r#"{"max_tokens": 64, "tools": []}"#),
        AnthropicHeaders::default(),
    );
    let fetcher = MapFetcher::default();
    let body = transformed(session, &request, &fetcher, Mode::Sync)
        .expect("the empty tools request transforms");
    assert_eq!(field(&body, "tools"), Some(&json("[]")));
}

fn malformed_tools(fixture: &Fixture, session: &mut Session<'_>) {
    let request = fixture.request(
        "claude-tools",
        prompt_message(),
        map_of(r#"{"max_tokens": 64, "tools": [42]}"#),
        AnthropicHeaders::default(),
    );
    let fetcher = MapFetcher::default();
    match transformed(session, &request, &fetcher, Mode::Sync) {
        Ok(_body) => panic!("the malformed tools entry must fail the request"),
        Err(ClaudeInvokeError::Tools(ToolSearchToolsError::NotObject { index })) => {
            assert_eq!(index, 0);
        }
        Err(other) => panic!("unexpected error: {other}"),
    }
}

pub(super) fn inline_document() {
    let fixture = plain_fixture();
    let mut session = Session::new(&fixture).expect("the session builds");
    text_document_inlined(&fixture, &mut session);
    document_sources(&fixture, &mut session);
}

fn document_sources(fixture: &Fixture, session: &mut Session<'_>) {
    let suffixed = messages_of(
        r#"[{"role": "user", "content": [
            {"type": "document", "source": {"type": "url", "url": "https://example.test/doc.pdf?token=1"}}
        ]}]"#,
    );
    let fetcher = MapFetcher::new(&[(
        "https://example.test/doc.pdf?token=1",
        "data:application/octet-stream;base64,JVBERi0xLjQK",
    )]);
    let body = plain_body(fixture, session, suffixed, &fetcher, Mode::Sync)
        .expect("the suffixed document transforms");
    assert_eq!(
        at(&body, &["messages", "0", "content", "0", "source"]),
        &json(
            r#"{"data": "JVBERi0xLjQK", "media_type": "application/octet-stream", "type": "base64"}"#,
        )
    );
    assert_eq!(
        fetcher.calls(),
        vec!["https://example.test/doc.pdf?token=1"]
    );
    let inline = messages_of(
        r#"[{"role": "user", "content": [
            {"type": "document", "source": {"type": "base64", "media_type": "application/pdf", "data": "QUJD"}}
        ]}]"#,
    );
    let quiet = MapFetcher::default();
    let inline_body = plain_body(fixture, session, inline, &quiet, Mode::Sync)
        .expect("the inline document transforms");
    assert_eq!(
        at(
            &inline_body,
            &["messages", "0", "content", "0", "source", "data"]
        ),
        &json(r#""QUJD""#)
    );
    assert!(quiet.calls().is_empty());
}

fn text_document_inlined(fixture: &Fixture, session: &mut Session<'_>) {
    let messages = messages_of(
        r#"[{"role": "user", "content": [
            {"type": "text", "text": "read this"},
            {"type": "document", "source": {"type": "url", "url": "https://example.test/doc.pdf"}}
        ]}]"#,
    );
    let fetcher = MapFetcher::new(&[(
        "https://example.test/doc.pdf",
        "data:application/octet-stream;base64,JVBERi0xLjQK",
    )]);
    let body = plain_body(fixture, session, messages, &fetcher, Mode::Sync)
        .expect("the document request transforms");
    assert_eq!(
        field(&body, "messages"),
        Some(&json(
            r#"[{
                "content": [
                    {"text": "read this", "type": "text"},
                    {"source": {"data": "JVBERi0xLjQK", "media_type": "application/pdf", "type": "base64"}, "type": "document"}
                ],
                "role": "user"
            }]"#,
        ))
    );
    assert_eq!(fetcher.calls(), vec!["https://example.test/doc.pdf"]);
}

pub(super) fn claude_response_usage() {
    let raw = r#"{
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
            "cache_read_input_tokens": 3
        }
    }"#;
    let envelope = response(raw, &prompt_message());
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
    let fixture = Fixture::new(&model_cost_of(TOOL_STUB_COST)).expect("the fixture builds");
    let mut session = Session::new(&fixture).expect("the session builds");
    structured_mapping(&mut session);
    display_update_params(&mut session);
    supported_param_list(&mut session);
}

fn structured_mapping(session: &mut Session<'_>) {
    let format = structured_params(RESPONSE_FORMAT);
    assert_eq!(
        map_params_of(session, "claude-test", &format, IndexMap::new()),
        json(
            r#"{
                "json_mode": true,
                "tool_choice": {"type": "tool", "name": "json_tool_call"},
                "tools": [{"name": "json_tool_call", "input_schema": {"type": "object", "properties": {"a": {"type": "string"}}, "required": ["a"]}}]
            }"#,
        )
    );
    assert_eq!(
        map_params_of(session, "claude-native", &format, IndexMap::new()),
        json(
            r#"{
                "json_mode": true,
                "output_format": {"type": "json_schema", "schema": {"additionalProperties": false, "type": "object", "properties": {"a": {"type": "string"}}, "required": ["a"]}}
            }"#,
        )
    );
}

fn display_update_params(session: &mut Session<'_>) {
    let requested =
        map_of(r#"{"thinking": {"type": "enabled", "display": "updates", "budget_tokens": 1024}}"#);
    let mut updates = requested.clone();
    session
        .map_params("claude-test", false, &requested, &mut updates)
        .expect("the display update maps");
    assert_eq!(
        at(&Json::Object(updates), &["thinking"]),
        &json(r#"{"type": "adaptive", "display": "updates"}"#)
    );
}

fn supported_param_list(session: &mut Session<'_>) {
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
    let fixture = Fixture::new(&model_cost_of(FORCED_COST)).expect("the fixture builds");
    let mut session = Session::new(&fixture).expect("the session builds");
    forced_choices(&mut session);
    kept_choices(&mut session);
}

fn forced_choices(session: &mut Session<'_>) {
    let json_tool =
        choice_params(r#"{"type": "function", "function": {"name": "json_tool_call"}}"#);
    assert_eq!(
        map_params_of(
            session,
            "claude-test",
            &structured_params(FORCED_FORMAT),
            json_tool.clone(),
        ),
        json(
            r#"{
                "json_mode": true,
                "tools": [{"name": "json_tool_call", "input_schema": {"type": "object", "properties": {}}}]
            }"#,
        )
    );
    assert_eq!(
        map_params_of(
            session,
            "claude-native",
            &structured_params(FORCED_FORMAT),
            json_tool,
        ),
        json(
            r#"{
                "json_mode": true,
                "tools": [{"name": "json_tool_call", "input_schema": {"type": "object", "properties": {}}}],
                "tool_choice": {"type": "tool", "name": "json_tool_call"}
            }"#,
        )
    );
}

fn kept_choices(session: &mut Session<'_>) {
    let weather = choice_params(r#"{"type": "function", "function": {"name": "get_weather"}}"#);
    let mapped = map_params_of(
        session,
        "claude-test",
        &structured_params(FORCED_FORMAT),
        weather,
    );
    assert_eq!(
        at(&mapped, &["tool_choice"]),
        &json(r#"{"type": "tool", "name": "get_weather"}"#)
    );
    let auto = map_params_of(
        session,
        "claude-test",
        &structured_params(FORCED_FORMAT),
        choice_params(r#""auto""#),
    );
    assert_eq!(at(&auto, &["tool_choice"]), &json(r#"{"type": "auto"}"#));
}

pub(super) fn remote_url_inlined() {
    let fixture = plain_fixture();
    let mut session = Session::new(&fixture).expect("the session builds");
    let messages = messages_of(
        r#"[{"role": "user", "content": [
            {"type": "image_url", "image_url": {"url": "https://example.test/pic.png"}}
        ]}]"#,
    );
    let fetcher = MapFetcher::new(&[(
        "https://example.test/pic.png",
        "data:image/png;base64,iVBORw0KGgo=",
    )]);
    let body = plain_body(&fixture, &mut session, messages, &fetcher, Mode::Async)
        .expect("the image request transforms");
    assert_eq!(
        field(&body, "messages"),
        Some(&json(
            r#"[{
                "content": [{
                    "source": {"data": "iVBORw0KGgo=", "media_type": "image/png", "type": "base64"},
                    "type": "image"
                }],
                "role": "user"
            }]"#,
        ))
    );
    assert_eq!(fetcher.calls(), vec!["https://example.test/pic.png"]);
}

pub(super) fn async_media_order() {
    assert_eq!([USES_ASYNC_TRANSFORM_REQUEST], [true]);
    let fixture = plain_fixture();
    let mut session = Session::new(&fixture).expect("the session builds");
    let messages = messages_of(
        r#"[{"role": "user", "content": [
            {"type": "document", "source": {"type": "url", "url": "https://example.test/second.pdf"}},
            {"type": "image_url", "image_url": "https://example.test/pic.png"},
            {"type": "text", "text": "and this"}
        ]}]"#,
    );
    let fetcher = mixed_media_fetcher();
    let body = plain_body(&fixture, &mut session, messages, &fetcher, Mode::Async)
        .expect("the mixed request transforms");
    assert_eq!(
        field(&body, "messages"),
        Some(&json(
            r#"[{
                "content": [
                    {"source": {"data": "U0VDT05E", "media_type": "application/pdf", "type": "base64"}, "type": "document"},
                    {"source": {"data": "iVBORw0KGgo=", "media_type": "image/png", "type": "base64"}, "type": "image"},
                    {"text": "and this", "type": "text"}
                ],
                "role": "user"
            }]"#,
        ))
    );
    assert_eq!(
        fetcher.calls(),
        vec![
            "https://example.test/second.pdf",
            "https://example.test/pic.png",
        ]
    );
}

fn mixed_media_fetcher() -> MapFetcher {
    MapFetcher::new(&[
        (
            "https://example.test/pic.png",
            "data:image/png;base64,iVBORw0KGgo=",
        ),
        (
            "https://example.test/second.pdf",
            "data:application/pdf;base64,U0VDT05E",
        ),
    ])
}

pub(super) fn media_fetch() {
    let fixture = plain_fixture();
    let mut session = Session::new(&fixture).expect("the session builds");
    served_media(&fixture, &mut session);
    refused_media(&fixture, &mut session);
}

const MEDIA_MESSAGES: &str = r#"[{"role": "user", "content": [
    {"type": "image_url", "image_url": "https://example.test/pic.png"},
    {"type": "document", "source": {"type": "url", "url": "https://example.test/doc.pdf"}}
]}]"#;

fn served_media(fixture: &Fixture, session: &mut Session<'_>) {
    let messages = messages_of(MEDIA_MESSAGES);
    let fetcher = MapFetcher::new(&[
        (
            "https://example.test/pic.png",
            "data:image/png;base64,iVBORw0KGgo=",
        ),
        (
            "https://example.test/doc.pdf",
            "data:application/octet-stream;base64,JVBERi0xLjQK",
        ),
    ]);
    let body = plain_body(fixture, session, messages, &fetcher, Mode::Async)
        .expect("the media request transforms");
    assert_eq!(
        at(&body, &["messages", "0", "content", "0", "source"]),
        &json(r#"{"data": "iVBORw0KGgo=", "media_type": "image/png", "type": "base64"}"#)
    );
    assert_eq!(
        at(&body, &["messages", "0", "content", "1", "source"]),
        &json(r#"{"data": "JVBERi0xLjQK", "media_type": "application/pdf", "type": "base64"}"#)
    );
    assert_eq!(fetcher.transport_attempts(), 2);
}

fn refused_media(fixture: &Fixture, session: &mut Session<'_>) {
    let messages = messages_of(MEDIA_MESSAGES);
    let fetcher = RefusingFetcher::default();
    match plain_body(fixture, session, messages, &fetcher, Mode::Async) {
        Ok(_body) => panic!("the refusing fetcher must fail the request"),
        Err(ClaudeInvokeError::Media(MediaError::Fetch(ImageFetchError::Rejected { url }))) => {
            assert_eq!(url, "https://example.test/pic.png");
        }
        Err(other) => panic!("unexpected error: {other}"),
    }
    assert_eq!(fetcher.transport_attempts(), 0);
    assert_eq!(fetcher.calls(), vec!["https://example.test/pic.png"]);
}

pub(super) fn structured_output_body() {
    let fixture = Fixture::new(&model_cost_of(OUTPUT_COST)).expect("the fixture builds");
    let mut session = Session::new(&fixture).expect("the session builds");
    native_output_config(&fixture, &mut session);
    unsupported_output_config(&fixture, &mut session);
}

fn native_output_config(fixture: &Fixture, session: &mut Session<'_>) {
    let mut params = map_of(r#"{"max_tokens": 64}"#);
    params.insert("output_format".to_owned(), json(OUTPUT_FORMAT));
    let request = fixture.request(
        "claude-native",
        prompt_message(),
        params,
        AnthropicHeaders::default(),
    );
    let fetcher = MapFetcher::default();
    let body = transformed(session, &request, &fetcher, Mode::Sync)
        .expect("the native structured output transforms");
    assert_eq!(
        at(&body, &["output_config", "format"]),
        &json(OUTPUT_FORMAT)
    );
    assert_eq!(
        field(&body, "anthropic_beta"),
        Some(&json(r#"["structured-outputs-2025-11-13"]"#))
    );
    assert!(field(&body, "output_format").is_none());
}

fn unsupported_output_config(fixture: &Fixture, session: &mut Session<'_>) {
    let mut output_config = IndexMap::new();
    output_config.insert("format".to_owned(), json(OUTPUT_FORMAT));
    output_config.insert("stream".to_owned(), Json::Bool(true));
    let mut params = map_of(r#"{"max_tokens": 64}"#);
    params.insert("output_config".to_owned(), Json::Object(output_config));
    let request = fixture.request(
        "claude-plain",
        prompt_message(),
        params,
        AnthropicHeaders::default(),
    );
    let fetcher = MapFetcher::default();
    let body = transformed(session, &request, &fetcher, Mode::Sync)
        .expect("the unsupported structured output transforms");
    assert!(field(&body, "output_config").is_none());
    assert_eq!(
        at(&body, &["messages", "0", "content", "0"]),
        &json(r#"{"text": "hello", "type": "text"}"#)
    );
    let inlined = at(&body, &["messages", "0", "content", "1", "text"])
        .as_str()
        .expect("the schema is inlined as text");
    assert_eq!(
        json(inlined),
        json(r#"{"type": "object", "properties": {"a": {"type": "string"}}, "required": ["a"]}"#)
    );
}

fn document_bodies<Url>(count: usize, url: Url) -> Vec<Json>
where
    Url: Fn(usize) -> String,
{
    let mut messages = Vec::with_capacity(count);
    for index in 0..count {
        let mut source = IndexMap::new();
        source.insert("type".to_owned(), Json::Text("url".to_owned()));
        source.insert("url".to_owned(), Json::Text(url(index)));
        let mut document = IndexMap::new();
        document.insert("type".to_owned(), Json::Text("document".to_owned()));
        document.insert("source".to_owned(), Json::Object(source));
        let mut user_message = IndexMap::new();
        user_message.insert("role".to_owned(), Json::Text("user".to_owned()));
        user_message.insert(
            "content".to_owned(),
            Json::List(vec![Json::Object(document)]),
        );
        messages.push(Json::Object(user_message));
    }
    messages
}

pub(super) fn remote_url_limit() {
    let fixture = plain_fixture();
    let mut session = Session::new(&fixture).expect("the session builds");
    let fetcher = MapFetcher::default();
    let over = plain_body(
        &fixture,
        &mut session,
        document_bodies(MAX_REMOTE_URLS + 1, |index| {
            format!("https://example.test/{index}.pdf")
        }),
        &fetcher,
        Mode::Sync,
    );
    assert_too_many(over);
    assert!(fetcher.calls().is_empty());
    let within = plain_body(
        &fixture,
        &mut session,
        document_bodies(MAX_REMOTE_URLS, |index| {
            format!("https://example.test/{index}.pdf")
        }),
        &fetcher,
        Mode::Sync,
    );
    expect_fetch_failure(within);
    repeated_urls(&fixture, &mut session);
}

fn assert_too_many(result: Result<Json, ClaudeInvokeError<Infallible>>) {
    match result {
        Ok(_body) => panic!("more than {MAX_REMOTE_URLS} document URLs must fail the request"),
        Err(ClaudeInvokeError::Documents(DocumentError::Media(
            MediaError::TooManyRemoteUrls { limit },
        ))) => {
            assert_eq!(limit, MAX_REMOTE_URLS);
        }
        Err(other) => panic!("unexpected error: {other}"),
    }
}

fn expect_fetch_failure(result: Result<Json, ClaudeInvokeError<Infallible>>) {
    match result {
        Ok(_body) => panic!("the empty fetcher cannot serve the document URLs"),
        Err(ClaudeInvokeError::Documents(DocumentError::Fetch(ImageFetchError::Attempts {
            ..
        }))) => {}
        Err(other) => panic!("unexpected error: {other}"),
    }
}

fn repeated_urls(fixture: &Fixture, session: &mut Session<'_>) {
    let quiet = MapFetcher::default();
    let repeated_over = plain_body(
        fixture,
        session,
        document_bodies(MAX_REMOTE_URLS + 1, |_| {
            "https://example.test/same.pdf".to_owned()
        }),
        &quiet,
        Mode::Sync,
    );
    assert_too_many(repeated_over);
    assert!(quiet.calls().is_empty());
    let served = MapFetcher::new(&[(
        "https://example.test/same.pdf",
        "data:application/pdf;base64,U0VN",
    )]);
    let at_ceiling = plain_body(
        fixture,
        session,
        document_bodies(MAX_REMOTE_URLS, |_| {
            "https://example.test/same.pdf".to_owned()
        }),
        &served,
        Mode::Sync,
    );
    if let Err(error) = at_ceiling {
        panic!("one URL repeated up to the ceiling must transform: {error}");
    }
    assert_eq!(served.calls().len(), MAX_REMOTE_URLS);
}

pub(super) fn billing_header() {
    assert_eq!([STRIPS_BILLING_METADATA], [true]);
    let fixture = plain_fixture();
    let mut session = Session::new(&fixture).expect("the session builds");
    let messages = messages_of(
        r#"[{"role": "system", "content": "x-anthropic-billing-header: abc"}, {"role": "user", "content": "hello"}]"#,
    );
    let fetcher = MapFetcher::default();
    let body = plain_body(&fixture, &mut session, messages, &fetcher, Mode::Sync)
        .expect("the billing request transforms");
    assert!(field(&body, "system").is_none());
}
