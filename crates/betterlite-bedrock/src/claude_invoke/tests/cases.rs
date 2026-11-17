use std::convert::Infallible;

use betterlite_anthropic::feature_flags::FeatureFlagsError;
use betterlite_anthropic::headers::AnthropicHeaders;
use betterlite_anthropic::params::error::ParamsError;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::support::{
    Fixture, MapFetcher, Mode, RefusingFetcher, Session, at, field, headers_of, json, map_of,
    messages_of, model_cost_of, plain_body, prompt_message, response, transformed,
};
use crate::claude_invoke::beta::InvokeBetaError;
use crate::claude_invoke::documents::DocumentError;
use crate::claude_invoke::error::ClaudeInvokeError;
use crate::claude_invoke::params::MapParamsError;
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
    "bedrock/claude-effort": {"litellm_provider": "bedrock", "supports_xhigh_reasoning_effort": true},
    "bedrock/claude-plain": {"litellm_provider": "bedrock"}
}"#;
const RESPONSE_FORMAT: &str = r#"{"type": "json_schema", "json_schema": {"name": "answer", "schema": {"type": "object", "properties": {"a": {"type": "string"}}, "required": ["a"]}}}"#;
const FORCED_FORMAT: &str = r#"{"type": "json_schema", "json_schema": {"name": "answer", "schema": {"type": "object", "properties": {}}}}"#;
const OUTPUT_FORMAT: &str = r#"{"type": "json_schema", "schema": {"type": "object", "properties": {"a": {"type": "string"}}, "required": ["a"]}}"#;
const BETA_COMPUTER_PARAMS: &str = r#"{"max_tokens": 64, "mcp_servers": [{"type": "url", "url": "https://example.test/mcp", "name": "srv"}], "tools": [{"type": "computer_20250124", "name": "computer", "display_width_px": 1024, "display_height_px": 768}]}"#;
const BETA_COMPUTER_EXPECTED: &str = r#"["computer-use-2025-01-24", "context-1m-2025-08-07"]"#;
const BETA_MCP_PARAMS: &str = r#"{"max_tokens": 64, "mcp_servers": [{"type": "url", "url": "https://example.test/mcp", "name": "srv"}]}"#;
const BETA_MID_MESSAGES: &str = r#"[{"role": "user", "content": "hello"}, {"role": "assistant", "content": "hi", "output_config": {"format": {"type": "json_schema", "schema": {"type": "object"}}}}]"#;
const BETA_MID_EXPECTED: &str = r#"["mid-conversation-output-config-2026-07-01"]"#;
const BETA_THINKING_PARAMS: &str =
    r#"{"max_tokens": 64, "thinking": {"type": "adaptive", "display": "updates"}}"#;
const BETA_THINKING_EXPECTED: &str = r#"["thinking-display-updates-2026-08-18"]"#;
const BETA_USER_EXPECTED: &str = r#"["mcp-client-2025-11-20"]"#;
const TOOL_SEARCH_TOOLS: &str = r#"[{"type": "tool_search_tool_regex_20251119"}, {"type": "tool_search_tool_regex_20251119", "name": "custom_search"}, {"type": "tool_search_tool_regex_20251119", "name": null, "input_schema": {"type": "object", "properties": {}}}, {"type": "tool_search_tool_bm25_20251119", "name": "search_bm25"}, {"name": "get_weather", "input_schema": {"type": "object", "properties": {}}, "eager_input_streaming": true, "custom": {"defer_loading": true}}]"#;
const TOOL_SEARCH_EXPECTED: &str = r#"[{"name": "tool_search_tool_regex", "type": "tool_search_tool_regex"}, {"name": "custom_search", "type": "tool_search_tool_regex"}, {"name": null, "type": "tool_search_tool_regex", "input_schema": {"type": "object", "properties": {}}}, {"name": "get_weather", "input_schema": {"type": "object", "properties": {}}, "defer_loading": true}]"#;
const TOOL_SEARCH_BETAS: &str =
    r#"["fine-grained-tool-streaming-2025-05-14", "tool-search-tool-2025-10-19"]"#;
const EAGER_TOOL: &str = r#"[{"type": "computer_20250124", "name": "computer", "display_width_px": 1024, "display_height_px": 768, "eager_input_streaming": "yes"}]"#;
const CHOICE_JSON_TOOL: &str = r#"{"type": "function", "function": {"name": "json_tool_call"}}"#;
const CHOICE_WEATHER: &str = r#"{"type": "function", "function": {"name": "get_weather"}}"#;
const CHOICE_STUB_BODY: &str = r#"{"json_mode": true, "tools": [{"name": "json_tool_call", "input_schema": {"type": "object", "properties": {}}}]}"#;
const CHOICE_NATIVE_BODY: &str = r#"{"json_mode": true, "tools": [{"name": "json_tool_call", "input_schema": {"type": "object", "properties": {}}}], "tool_choice": {"type": "tool", "name": "json_tool_call"}}"#;
const CHOICE_WEATHER_BODY: &str = r#"{"json_mode": true, "tools": [{"name": "json_tool_call", "input_schema": {"type": "object", "properties": {}}}], "tool_choice": {"type": "tool", "name": "get_weather"}}"#;
const CHOICE_AUTO_BODY: &str = r#"{"json_mode": true, "tools": [{"name": "json_tool_call", "input_schema": {"type": "object", "properties": {}}}], "tool_choice": {"type": "auto"}}"#;

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

fn model_body(
    fixture: &Fixture,
    session: &mut Session<'_>,
    model: &str,
    messages: Vec<Json>,
    params: IndexMap<String, Json>,
) -> Json {
    let request = fixture.request(model, messages, params, AnthropicHeaders::default());
    let fetcher = MapFetcher::default();
    transformed(session, &request, &fetcher, Mode::Sync).expect("the request transforms")
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
    let params = map_of(r#"{"max_tokens": 64, "output_config": {"effort": "xhigh"}}"#);
    let caller_params = Json::Object(params.clone());
    let body = model_body(fixture, session, "claude-config", prompt_message(), params);
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
    beta_scenarios(&fixture, &mut session);
    effort_beta(&fixture, &mut session);
    eager_tool_beta();
    mcp_beta();
}

fn effort_beta(fixture: &Fixture, session: &mut Session<'_>) {
    let params = map_of(r#"{"max_tokens": 64, "output_config": {"effort": "high"}}"#);
    let beta = beta_of(
        fixture,
        session,
        prompt_message(),
        params,
        AnthropicHeaders::default(),
    );
    assert_eq!(beta, Some(json(r#"["effort-2025-11-24"]"#)));
}

fn eager_tool_beta() {
    let fixture = Fixture::new(&model_cost_of(TOOL_COST)).expect("the fixture builds");
    let mut session = Session::new(&fixture).expect("the session builds");
    let error = error_of(tool_body(&fixture, &mut session, json(EAGER_TOOL)));
    assert_eq!(error.status_code(), Some(400));
    assert!(matches!(
        error,
        ClaudeInvokeError::Beta(InvokeBetaError::Features(
            FeatureFlagsError::EagerInputStreaming
        ))
    ));
}

fn mcp_beta() {
    let mut fixture = plain_fixture();
    fixture.keep_mcp_betas();
    let mut session = Session::new(&fixture).expect("the session builds");
    let request = fixture.request(
        "claude-plain",
        prompt_message(),
        map_of(BETA_MCP_PARAMS),
        AnthropicHeaders::default(),
    );
    let fetcher = MapFetcher::default();
    let body = transformed(&mut session, &request, &fetcher, Mode::Sync)
        .expect("the mcp beta request transforms");
    assert_eq!(
        field(&body, "anthropic_beta"),
        Some(&json(r#"["mcp-client-2025-04-04"]"#))
    );
}

fn beta_scenarios(fixture: &Fixture, session: &mut Session<'_>) {
    let scenarios = [
        (
            prompt_message(),
            map_of(BETA_COMPUTER_PARAMS),
            headers_of(&[("anthropic-beta", "context-1m-2025-08-07")]),
            Some(json(BETA_COMPUTER_EXPECTED)),
        ),
        (
            prompt_message(),
            map_of(BETA_MCP_PARAMS),
            AnthropicHeaders::default(),
            None,
        ),
        (
            prompt_message(),
            map_of(r#"{"max_tokens": 64}"#),
            headers_of(&[("anthropic-beta", "mcp-client-2025-11-20")]),
            Some(json(BETA_USER_EXPECTED)),
        ),
        (
            messages_of(BETA_MID_MESSAGES),
            map_of(r#"{"max_tokens": 64}"#),
            AnthropicHeaders::default(),
            Some(json(BETA_MID_EXPECTED)),
        ),
        (
            prompt_message(),
            map_of(BETA_THINKING_PARAMS),
            AnthropicHeaders::default(),
            Some(json(BETA_THINKING_EXPECTED)),
        ),
    ];
    for (messages, params, headers, expected) in scenarios {
        assert_eq!(
            beta_of(fixture, session, messages, params, headers),
            expected
        );
    }
}

fn beta_of(
    fixture: &Fixture,
    session: &mut Session<'_>,
    messages: Vec<Json>,
    params: IndexMap<String, Json>,
    headers: AnthropicHeaders,
) -> Option<Json> {
    let request = fixture.request("claude-plain", messages, params, headers);
    let fetcher = MapFetcher::default();
    let body =
        transformed(session, &request, &fetcher, Mode::Sync).expect("the beta request transforms");
    field(&body, "anthropic_beta").cloned()
}

pub(super) fn tool_search_tool_name() {
    let fixture = Fixture::new(&model_cost_of(TOOL_COST)).expect("the fixture builds");
    let mut session = Session::new(&fixture).expect("the session builds");
    tool_search_cases(&fixture, &mut session);
}

fn tool_body(
    fixture: &Fixture,
    session: &mut Session<'_>,
    tools: Json,
) -> Result<Json, ClaudeInvokeError<Infallible>> {
    let mut params = map_of(r#"{"max_tokens": 64}"#);
    params.insert("tools".to_owned(), tools);
    let request = fixture.request(
        "claude-tools",
        prompt_message(),
        params,
        AnthropicHeaders::default(),
    );
    let fetcher = MapFetcher::default();
    transformed(session, &request, &fetcher, Mode::Sync)
}

fn tool_search_cases(fixture: &Fixture, session: &mut Session<'_>) {
    let named = tool_body(fixture, session, json(TOOL_SEARCH_TOOLS))
        .expect("the tool search request transforms");
    assert_eq!(field(&named, "tools"), Some(&json(TOOL_SEARCH_EXPECTED)));
    assert_eq!(
        field(&named, "anthropic_beta"),
        Some(&json(TOOL_SEARCH_BETAS))
    );
    let empty =
        tool_body(fixture, session, json("[]")).expect("the empty tools request transforms");
    assert_eq!(field(&empty, "tools"), Some(&json("[]")));
    match tool_body(fixture, session, json("[42]")) {
        Ok(_body) => panic!("the malformed tools entry must fail the request"),
        Err(ClaudeInvokeError::Tools(error)) => {
            assert_eq!(error, ToolSearchToolsError::NotObject { index: 0 });
            assert_eq!(error.status_code(), 400);
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
    data_url_fetched(fixture, session);
}

fn data_url_fetched(fixture: &Fixture, session: &mut Session<'_>) {
    let fetched = MapFetcher::new(&[("data:text/plain,plain", "data:application/pdf;base64,U0VN")]);
    let plain = document_bodies(1, |_| "data:text/plain,plain".to_owned());
    let body = plain_body(fixture, session, plain, &fetched, Mode::Sync)
        .expect("the non-base64 data URL is fetched");
    assert_eq!(
        body,
        json(
            r#"{"anthropic_version": "bedrock-2023-05-31", "max_tokens": 64, "messages": [{"content": [{"source": {"data": "U0VN", "media_type": "application/pdf", "type": "base64"}, "type": "document"}], "role": "user"}]}"#,
        )
    );
    assert_eq!(fetched.calls(), vec!["data:text/plain,plain"]);
    let quiet = MapFetcher::default();
    let inline = document_bodies(1, |_| "data:application/pdf;base64,QUJD".to_owned());
    let unchanged = plain_body(fixture, session, inline, &quiet, Mode::Sync)
        .expect("the base64 data URL passes through");
    assert_eq!(
        unchanged,
        json(
            r#"{"anthropic_version": "bedrock-2023-05-31", "max_tokens": 64, "messages": [{"content": [{"source": {"data": "QUJD", "media_type": "application/pdf", "type": "base64"}, "type": "document"}], "role": "user"}]}"#,
        )
    );
    assert!(quiet.calls().is_empty());
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
    parameter_error_status(&mut session);
    pre_mapped_thinking(&mut session);
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

fn parameter_error_status(session: &mut Session<'_>) {
    let mut optional_params = IndexMap::new();
    let error = session
        .map_params(
            "claude-test",
            false,
            &map_of(r#"{"max_tokens": "many"}"#),
            &mut optional_params,
        )
        .expect_err("a text max_tokens must fail the mapping");
    assert_eq!(error.status_code(), Some(400));
    assert!(matches!(
        error,
        MapParamsError::Params(ParamsError::NotNumber {
            field: "max_tokens"
        })
    ));
}

fn pre_mapped_thinking(session: &mut Session<'_>) {
    let mut optional_params = IndexMap::new();
    optional_params.insert(
        "thinking".to_owned(),
        json(r#"{"type": "enabled", "budget_tokens": 1024, "display": "summarized"}"#),
    );
    let mapped = map_params_of(
        session,
        "claude-test",
        &structured_params(RESPONSE_FORMAT),
        optional_params,
    );
    assert_eq!(
        field(&mapped, "thinking"),
        Some(&json(r#"{"type": "adaptive", "display": "summarized"}"#))
    );
}

pub(super) fn forced_tool_choice_removed() {
    let fixture = Fixture::new(&model_cost_of(FORCED_COST)).expect("the fixture builds");
    let mut session = Session::new(&fixture).expect("the session builds");
    choice_scenarios(&mut session);
}

fn choice_scenarios(session: &mut Session<'_>) {
    let scenarios = [
        ("claude-test", CHOICE_JSON_TOOL, CHOICE_STUB_BODY),
        ("claude-native", CHOICE_JSON_TOOL, CHOICE_NATIVE_BODY),
        ("claude-test", CHOICE_WEATHER, CHOICE_WEATHER_BODY),
        ("claude-test", r#""auto""#, CHOICE_AUTO_BODY),
    ];
    for (model, choice, expected) in scenarios {
        assert_eq!(
            map_params_of(
                session,
                model,
                &structured_params(FORCED_FORMAT),
                choice_params(choice),
            ),
            json(expected)
        );
    }
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
    media_scenarios(&fixture, &mut session);
}

const MEDIA_MESSAGES: &str = r#"[{"role": "user", "content": [
    {"type": "image_url", "image_url": "https://example.test/pic.png"},
    {"type": "document", "source": {"type": "url", "url": "https://example.test/doc.pdf"}}
]}]"#;

const SERVED_MEDIA: &[(&str, &str)] = &[
    (
        "https://example.test/pic.png",
        "data:image/png;base64,iVBORw0KGgo=",
    ),
    (
        "https://example.test/doc.pdf",
        "data:application/octet-stream;base64,JVBERi0xLjQK",
    ),
];

fn media_scenarios(fixture: &Fixture, session: &mut Session<'_>) {
    let served = MapFetcher::new(SERVED_MEDIA);
    let body = plain_body(
        fixture,
        session,
        messages_of(MEDIA_MESSAGES),
        &served,
        Mode::Async,
    )
    .expect("the media request transforms");
    assert_eq!(
        at(&body, &["messages", "0", "content", "0", "source"]),
        &json(r#"{"data": "iVBORw0KGgo=", "media_type": "image/png", "type": "base64"}"#)
    );
    assert_eq!(
        at(&body, &["messages", "0", "content", "1", "source"]),
        &json(r#"{"data": "JVBERi0xLjQK", "media_type": "application/pdf", "type": "base64"}"#)
    );
    assert_eq!(served.transport_attempts(), 2);
    let refusing = RefusingFetcher::default();
    match plain_body(
        fixture,
        session,
        messages_of(MEDIA_MESSAGES),
        &refusing,
        Mode::Async,
    ) {
        Ok(_body) => panic!("the refusing fetcher must fail the request"),
        Err(ClaudeInvokeError::Media(MediaError::Fetch(ImageFetchError::Rejected { url }))) => {
            assert_eq!(url, "https://example.test/pic.png");
        }
        Err(other) => panic!("unexpected error: {other}"),
    }
    assert_eq!(refusing.transport_attempts(), 0);
    assert_eq!(refusing.calls(), vec!["https://example.test/pic.png"]);
}

pub(super) fn structured_output_body() {
    let fixture = Fixture::new(&model_cost_of(OUTPUT_COST)).expect("the fixture builds");
    let mut session = Session::new(&fixture).expect("the session builds");
    let mut params = map_of(r#"{"max_tokens": 64}"#);
    params.insert("output_format".to_owned(), json(OUTPUT_FORMAT));
    let body = model_body(
        &fixture,
        &mut session,
        "claude-native",
        prompt_message(),
        params,
    );
    assert_eq!(
        at(&body, &["output_config", "format"]),
        &json(OUTPUT_FORMAT)
    );
    assert_eq!(
        field(&body, "anthropic_beta"),
        Some(&json(r#"["structured-outputs-2025-11-13"]"#))
    );
    assert!(field(&body, "output_format").is_none());
    unsupported_output_config(&fixture, &mut session);
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
    let Json::Text(inlined) = at(&body, &["messages", "0", "content", "1", "text"]) else {
        panic!("the schema is inlined as text");
    };
    assert_eq!(
        json(inlined),
        json(r#"{"type": "object", "properties": {"a": {"type": "string"}}, "required": ["a"]}"#)
    );
    let kept = model_body(
        fixture,
        session,
        "claude-effort",
        prompt_message(),
        map_of(r#"{"max_tokens": 64, "output_config": {"effort": "high"}}"#),
    );
    assert_eq!(
        field(&kept, "output_config"),
        Some(&json(r#"{"effort": "high"}"#))
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
    let error = error_of(over);
    assert_eq!(error.status_code(), Some(400));
    assert!(matches!(
        error,
        ClaudeInvokeError::Documents(DocumentError::Media(MediaError::TooManyRemoteUrls {
            limit: MAX_REMOTE_URLS,
        }))
    ));
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
    assert!(matches!(
        error_of(within),
        ClaudeInvokeError::Documents(DocumentError::Fetch(ImageFetchError::Attempts { .. }))
    ));
    repeated_urls(&fixture, &mut session);
}

fn error_of(result: Result<Json, ClaudeInvokeError<Infallible>>) -> ClaudeInvokeError<Infallible> {
    match result {
        Ok(_body) => panic!("the request must fail"),
        Err(error) => error,
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
    assert!(matches!(
        error_of(repeated_over),
        ClaudeInvokeError::Documents(DocumentError::Media(MediaError::TooManyRemoteUrls {
            limit: MAX_REMOTE_URLS,
        }))
    ));
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
