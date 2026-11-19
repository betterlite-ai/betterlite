use betterlite_config::sections::bedrock_services_search::{
    DEFAULT_MCP_PROTOCOL_VERSION, DEFAULT_SEARCH_TOOL_NAME,
};
use betterlite_openai::search::provider::SearchQuery;

use crate::search::UI_FRIENDLY_NAME;
use crate::search::headers::{ACCEPT, CONTENT_TYPE, validate_environment};
use crate::search::request::{Params, transform_search_request};
use crate::search::tests::support::{header, headers, params, text_field};
use crate::search::url::complete_url;

pub(super) fn assert_configured_gateway_required() {
    assert_eq!(UI_FRIENDLY_NAME, "Web Search on Amazon Bedrock");
    assert_explicit_api_base_wins();
    assert_configured_url_is_verbatim();
    assert_missing_gateway_is_refused();
}

fn assert_explicit_api_base_wins() {
    let explicit = "https://explicit.example.com/mcp";
    let configured = Some("https://configured.example.com/mcp");
    assert_eq!(complete_url(Some(explicit), configured).unwrap(), explicit);
    assert_eq!(complete_url(Some(" "), configured).unwrap(), " ");
}

// no /mcp append and no trim: the configured value is returned exactly as written
fn assert_configured_url_is_verbatim() {
    let configured = "https://gateway-abc123.gateway.bedrock-agentcore.us-east-1.amazonaws.com";
    assert_eq!(complete_url(None, Some(configured)).unwrap(), configured);
    assert_eq!(
        complete_url(Some(""), Some(configured)).unwrap(),
        configured
    );
}

fn assert_missing_gateway_is_refused() {
    let expected = "AGENTCORE_GATEWAY_URL is not set. Set it to your AgentCore Gateway MCP \
                    endpoint (https://<gateway-id>.gateway.bedrock-agentcore.<region>\
                    .amazonaws.com/mcp) or pass api_base.";
    assert_eq!(complete_url(None, None).unwrap_err().to_string(), expected);
    assert_eq!(
        complete_url(Some(""), Some("")).unwrap_err().to_string(),
        expected
    );
}

pub(super) fn assert_mcp_request_headers() {
    assert_overlay_replaces_exact_keys();
    assert_overlay_is_case_exact();
    assert_protocol_version_is_configurable();
    assert_unrelated_headers_survive();
}

fn assert_overlay_replaces_exact_keys() {
    let supplied = headers(&[
        ("Content-Type", "text/plain"),
        ("Accept", "text/html"),
        ("MCP-Protocol-Version", "2024-11-05"),
    ]);
    let merged = validate_environment(supplied, None);
    assert_eq!(CONTENT_TYPE, "application/json");
    assert_eq!(ACCEPT, "application/json, text/event-stream");
    assert_eq!(header(&merged, "Content-Type"), Some("application/json"));
    assert_eq!(
        header(&merged, "Accept"),
        Some("application/json, text/event-stream")
    );
    assert_eq!(
        header(&merged, "MCP-Protocol-Version"),
        Some(DEFAULT_MCP_PROTOCOL_VERSION)
    );
    assert_eq!(header(&merged, "MCP-Protocol-Version"), Some("2025-03-26"));
    assert_eq!(merged.len(), 3);
}

fn assert_overlay_is_case_exact() {
    let supplied = headers(&[("content-type", "text/plain")]);
    let merged = validate_environment(supplied, None);
    assert_eq!(header(&merged, "content-type"), Some("text/plain"));
    assert_eq!(header(&merged, "Content-Type"), Some("application/json"));
    assert_eq!(merged.len(), 4);
}

fn assert_protocol_version_is_configurable() {
    let configured = validate_environment(headers(&[]), Some("2025-06-18"));
    assert_eq!(
        header(&configured, "MCP-Protocol-Version"),
        Some("2025-06-18")
    );
    let empty = validate_environment(headers(&[]), Some(""));
    assert_eq!(
        header(&empty, "MCP-Protocol-Version"),
        Some(DEFAULT_MCP_PROTOCOL_VERSION)
    );
    let blank = validate_environment(headers(&[]), Some(" "));
    assert_eq!(header(&blank, "MCP-Protocol-Version"), Some(" "));
}

fn assert_unrelated_headers_survive() {
    let supplied = headers(&[("x-request-id", "abc"), ("anthropic-version", "2023-06-01")]);
    let merged = validate_environment(supplied, None);
    assert_eq!(header(&merged, "x-request-id"), Some("abc"));
    assert_eq!(header(&merged, "anthropic-version"), Some("2023-06-01"));
    assert_eq!(merged.len(), 5);
}

pub(super) fn assert_websearch_tool_suffix() {
    assert_default_tool_name();
    assert_configured_tool_name();
    assert_explicit_tool_name_wins();
    assert_suffix_alone_is_accepted();
    assert_missing_suffix_is_refused();
    assert_miscased_suffix_is_refused();
    assert_non_text_tool_name_is_refused();
}

fn single() -> SearchQuery {
    SearchQuery::Single("hello".to_owned())
}

fn tool_name(query: &SearchQuery, optional: &Params, configured: Option<&str>) -> String {
    let body = transform_search_request(query, optional, configured).unwrap();
    text_field(&body, &["params", "name"]).to_owned()
}

fn assert_default_tool_name() {
    let selected = tool_name(&single(), &Params::new(), None);
    assert_eq!(selected, "web-search-tool___WebSearch");
    assert_eq!(selected, DEFAULT_SEARCH_TOOL_NAME);
}

fn assert_configured_tool_name() {
    let configured = Some("custom-target___WebSearch");
    assert_eq!(
        tool_name(&single(), &Params::new(), configured),
        "custom-target___WebSearch"
    );
    assert_eq!(
        tool_name(&single(), &Params::new(), Some("")),
        DEFAULT_SEARCH_TOOL_NAME
    );
}

fn assert_explicit_tool_name_wins() {
    let configured = Some("custom-target___WebSearch");
    let explicit = params(r#"{"tool_name":"packages___WebSearch"}"#);
    assert_eq!(
        tool_name(&single(), &explicit, configured),
        "packages___WebSearch"
    );
    let empty = params(r#"{"tool_name":""}"#);
    assert_eq!(
        tool_name(&single(), &empty, configured),
        "custom-target___WebSearch"
    );
    let null = params(r#"{"tool_name":null}"#);
    assert_eq!(
        tool_name(&single(), &null, configured),
        "custom-target___WebSearch"
    );
}

fn assert_suffix_alone_is_accepted() {
    let optional = params(r#"{"tool_name":"___WebSearch"}"#);
    assert_eq!(tool_name(&single(), &optional, None), "___WebSearch");
}

fn tool_error(name: &str) -> String {
    format!(
        "Invalid AgentCore search tool_name '{name}': must end with '___WebSearch' (a \
         web-search connector tool). Other gateway tools cannot be invoked through this provider."
    )
}

fn assert_missing_suffix_is_refused() {
    let optional = params(r#"{"tool_name":"web-search-tool"}"#);
    let error = transform_search_request(&single(), &optional, None).unwrap_err();
    assert_eq!(error.to_string(), tool_error("web-search-tool"));
    let embedded = params(r#"{"tool_name":"a___WebSearchb"}"#);
    let error = transform_search_request(&single(), &embedded, None).unwrap_err();
    assert_eq!(error.to_string(), tool_error("a___WebSearchb"));
    let error =
        transform_search_request(&single(), &Params::new(), Some("custom-target")).unwrap_err();
    assert_eq!(error.to_string(), tool_error("custom-target"));
}

fn assert_miscased_suffix_is_refused() {
    let optional = params(r#"{"tool_name":"web-search-tool___websearch"}"#);
    let error = transform_search_request(&single(), &optional, None).unwrap_err();
    assert_eq!(error.to_string(), tool_error("web-search-tool___websearch"));
}

fn assert_non_text_tool_name_is_refused() {
    let optional = params(r#"{"tool_name":7}"#);
    let error = transform_search_request(&single(), &optional, None).unwrap_err();
    assert_eq!(error.to_string(), "tool_name must be a string");
}
