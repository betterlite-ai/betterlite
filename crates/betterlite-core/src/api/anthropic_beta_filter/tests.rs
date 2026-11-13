#![cfg(test)]

use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::Headers;
use super::error::BetaFilterError;
use super::matrix_filter::{
    BetaDropReason, DroppedBeta, filter_beta_tokens, filter_header_betas, filter_request_betas,
};
use super::native_injection::{NativeRequest, inject_native_betas};
use crate::api::anthropic_beta_config::matrix::BetaMatrix;

const MATRIX: &str = r#"{
    "anthropic": {
        "effort-2025-11-24": "effort-2025-11-24",
        "computer-use-2025-01-24": "computer-use-2025-01-24"
    },
    "bedrock": {
        "advanced-tool-use-2025-11-20": "tool-search-tool-2025-10-19",
        "fast-mode-2026-02-01": null
    },
    "provider_aliases": {"alias": "anthropic"}
}"#;

fn matrix(source: &str) -> BetaMatrix {
    serde_json::from_str(source).unwrap()
}

fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn params(source: &str) -> IndexMap<String, Json> {
    serde_json::from_str(source).unwrap()
}

fn messages(source: &str) -> Vec<Json> {
    serde_json::from_str(source).unwrap()
}

fn drops(pairs: &[(&str, BetaDropReason)]) -> Vec<DroppedBeta> {
    pairs
        .iter()
        .map(|(token, reason)| DroppedBeta {
            token: (*token).to_owned(),
            reason: *reason,
        })
        .collect()
}

fn inject(
    pairs: &[(&str, &str)],
    provider: &str,
    params_json: &str,
    messages_json: &str,
) -> Headers {
    let optional_params = params(params_json);
    let request_messages = messages(messages_json);
    inject_native_betas(
        headers(pairs),
        &NativeRequest {
            provider,
            optional_params: &optional_params,
            messages: &request_messages,
        },
    )
    .unwrap()
}

fn beta_header(headers: &Headers) -> Option<&str> {
    headers.get("anthropic-beta").map(String::as_str)
}

#[rstest]
#[case::bedrock_maps_and_drops(
    "bedrock",
    &[
        " advanced-tool-use-2025-11-20 ",
        "tool-search-tool-2025-10-19",
        "fast-mode-2026-02-01",
        "invented"
    ],
    &["tool-search-tool-2025-10-19"],
    &[
        ("tool-search-tool-2025-10-19", BetaDropReason::Unknown),
        ("fast-mode-2026-02-01", BetaDropReason::Unsupported),
        ("invented", BetaDropReason::Unknown)
    ]
)]
#[case::unknown_provider_drops_every_token(
    "nowhere",
    &["effort-2025-11-24", "computer-use-2025-01-24"],
    &[],
    &[
        ("effort-2025-11-24", BetaDropReason::Unknown),
        ("computer-use-2025-01-24", BetaDropReason::Unknown)
    ]
)]
#[case::alias_maps_two_tokens_to_one(
    "alias",
    &["effort-2025-11-24", " computer-use-2025-01-24", "effort-2025-11-24"],
    &["computer-use-2025-01-24", "effort-2025-11-24"],
    &[]
)]
fn mapped_beta_tokens(
    #[case] provider: &str,
    #[case] case_tokens: &[&str],
    #[case] expected: &[&str],
    #[case] expected_drops: &[(&str, BetaDropReason)],
) {
    let tokens: Vec<String> = case_tokens
        .iter()
        .map(|token| (*token).to_owned())
        .collect();
    let filtered = filter_beta_tokens(&matrix(MATRIX), provider, &tokens).unwrap();
    assert_eq!(filtered.tokens, expected);
    assert_eq!(filtered.dropped, drops(expected_drops));
}

#[test]
fn empty_input_never_reads_a_broken_provider_mapping() {
    let broken = matrix(r#"{"anthropic": "not a mapping"}"#);
    let filtered = filter_beta_tokens(&broken, "anthropic", &[]).unwrap();
    assert!(filtered.tokens.is_empty());
    assert!(filtered.dropped.is_empty());
}

#[test]
fn header_body_removal() {
    let mut request_headers = headers(&[
        ("anthropic-beta", "unknown,, "),
        ("X-Keep", "yes"),
        ("Anthropic-Beta", "kept"),
    ]);
    let mut body: IndexMap<String, Json> =
        params(r#"{"anthropic_beta": ["unknown"], "keep": true}"#);
    let dropped = filter_request_betas(
        &mut request_headers,
        &mut body,
        &matrix(MATRIX),
        "anthropic",
    )
    .unwrap();
    assert_eq!(
        dropped,
        drops(&[
            ("unknown", BetaDropReason::Unknown),
            ("unknown", BetaDropReason::Unknown)
        ])
    );
    assert_eq!(request_headers.len(), 2);
    assert_eq!(
        request_headers.get("X-Keep").map(String::as_str),
        Some("yes")
    );
    assert_eq!(
        request_headers.get("Anthropic-Beta").map(String::as_str),
        Some("kept")
    );
    assert_eq!(body.len(), 1);
    assert_eq!(body.get("keep"), Some(&Json::Bool(true)));
}

#[test]
fn absent_and_empty_headers_and_body_fields() {
    let matrix = matrix(MATRIX);
    let mut absent = headers(&[("X-Keep", "yes")]);
    let mut kept: IndexMap<String, Json> = params(r#"{"anthropic_beta": [], "keep": true}"#);
    let dropped = filter_request_betas(&mut absent, &mut kept, &matrix, "anthropic").unwrap();
    assert!(dropped.is_empty());
    assert_eq!(absent, headers(&[("X-Keep", "yes")]));
    assert_eq!(kept.get("anthropic_beta"), Some(&Json::List(Vec::new())));

    let mut null_body: IndexMap<String, Json> = params(r#"{"anthropic_beta": null}"#);
    let mut empty = headers(&[("anthropic-beta", "")]);
    let empty_drops =
        filter_request_betas(&mut empty, &mut null_body, &matrix, "anthropic").unwrap();
    assert!(empty_drops.is_empty());
    assert!(empty.is_empty());
    assert_eq!(null_body.get("anthropic_beta"), Some(&Json::Null));
}

#[test]
fn mixed_header_tokens_are_sorted_without_spaces() {
    let mut request_headers = headers(&[(
        "anthropic-beta",
        "effort-2025-11-24, computer-use-2025-01-24,effort-2025-11-24",
    )]);
    let dropped = filter_header_betas(&mut request_headers, &matrix(MATRIX), "anthropic").unwrap();
    assert!(dropped.is_empty());
    assert_eq!(
        beta_header(&request_headers),
        Some("computer-use-2025-01-24,effort-2025-11-24")
    );
}

#[test]
fn native_beta_merge() {
    let merged = inject(
        &[
            ("Anthropic-Beta", " custom ,custom"),
            ("anthropic-beta", "other"),
            ("X-Keep", "yes"),
        ],
        "anthropic",
        "{}",
        "[]",
    );
    assert_eq!(merged.len(), 2);
    assert_eq!(merged.get("X-Keep").map(String::as_str), Some("yes"));
    assert_eq!(beta_header(&merged), Some("custom,other"));
}

#[test]
fn native_merge_leaves_empty_requests_unchanged() {
    assert!(inject(&[], "anthropic", "{}", "[]").is_empty());
    let empty_beta = inject(&[("Anthropic-Beta", "")], "anthropic", "{}", "[]");
    assert_eq!(empty_beta, headers(&[("Anthropic-Beta", "")]));
}

#[rstest]
#[case::no_edits("{}", "[]", "anthropic", None)]
#[case::empty_edits(r#"{"context_management": {"edits": []}}"#, "[]", "anthropic", None)]
#[case::compact_edit(
    r#"{"context_management": {"edits": [{"type": "compact_20260112"}]}}"#,
    "[]",
    "anthropic",
    Some("compact-2026-01-12")
)]
#[case::other_edit(
    r#"{"context_management": {"edits": [{"type": "clear_tool_uses_20250919"}]}}"#,
    "[]",
    "anthropic",
    Some("context-management-2025-06-27")
)]
#[case::edit_without_a_type(
    r#"{"context_management": {"edits": [{"type": "clear_tool_uses_20250919"}, {}]}}"#,
    "[]",
    "anthropic",
    Some("context-management-2025-06-27")
)]
#[case::both_edits(
    r#"{"context_management": {"edits": [{"type": "compact_20260112"}, {"type": "other"}]}}"#,
    "[]",
    "anthropic",
    Some("compact-2026-01-12,context-management-2025-06-27")
)]
#[case::native_compaction(r#"{"compaction": {}}"#, "[]", "anthropic", Some("compact-2026-09-04"))]
#[case::native_compaction_for_bedrock(r#"{"compaction": {}}"#, "[]", "bedrock", None)]
#[case::signed_compaction_block(
    "{}",
    r#"[{"role": "user", "content": [{"type": "compaction", "signature": "sig"}]}]"#,
    "anthropic",
    Some("compact-2026-09-04")
)]
#[case::unsigned_compaction_block(
    "{}",
    r#"[{"role": "user", "content": [{"type": "compaction", "signature": ""}]}]"#,
    "anthropic",
    None
)]
#[case::signed_compaction_block_for_bedrock(
    "{}",
    r#"[{"role": "user", "content": [{"type": "compaction", "signature": "sig"}]}]"#,
    "bedrock",
    None
)]
fn context_compaction_betas(
    #[case] optional_params: &str,
    #[case] request_messages: &str,
    #[case] provider: &str,
    #[case] expected: Option<&str>,
) {
    let merged = inject(&[], provider, optional_params, request_messages);
    assert_eq!(beta_header(&merged), expected);
}

#[rstest]
#[case::output_format(
    r#"{"output_format": {}}"#,
    "[]",
    Some("structured-outputs-2025-11-13")
)]
#[case::nested_output_config(
    r#"{"output_config": {"format": {}}}"#,
    "[]",
    Some("structured-outputs-2025-11-13")
)]
#[case::null_nested_format(r#"{"output_config": {"format": null}}"#, "[]", None)]
#[case::output_config_without_format(r#"{"output_config": {}}"#, "[]", None)]
#[case::null_output_format(r#"{"output_format": null}"#, "[]", None)]
#[case::fast_speed(r#"{"speed": "fast"}"#, "[]", Some("fast-mode-2026-02-01"))]
#[case::standard_speed(r#"{"speed": "standard"}"#, "[]", None)]
#[case::message_output_config(
    "{}",
    r#"[{"role": "user", "content": "hi", "output_config": null}]"#,
    Some("per-turn-control-2026-07-01")
)]
fn output_and_speed_betas(
    #[case] optional_params: &str,
    #[case] request_messages: &str,
    #[case] expected: Option<&str>,
) {
    let merged = inject(&[], "anthropic", optional_params, request_messages);
    assert_eq!(beta_header(&merged), expected);
}

#[rstest]
#[case::advisor_tool(
    "anthropic",
    r#"{"tools": [{"type": "advisor_20260301"}]}"#,
    "[]",
    Some("advisor-tool-2026-03-01")
)]
#[case::regex_tool_search_for_vertex_ai(
    "vertex_ai",
    r#"{"tools": [{"type": "tool_search_tool_regex_20251119"}]}"#,
    "[]",
    Some("tool-search-tool-2025-10-19")
)]
#[case::regex_tool_search_for_vertex_ai_beta(
    "vertex_ai_beta",
    r#"{"tools": [{"type": "tool_search_tool_regex_20251119"}]}"#,
    "[]",
    Some("tool-search-tool-2025-10-19")
)]
#[case::bm25_tool_search_for_bedrock(
    "bedrock",
    r#"{"tools": [{"type": "tool_search_tool_bm25_20251119"}]}"#,
    "[]",
    Some("tool-search-tool-2025-10-19")
)]
#[case::tool_search_for_anthropic(
    "anthropic",
    r#"{"tools": [{"type": "tool_search_tool_regex_20251119"}]}"#,
    "[]",
    Some("advanced-tool-use-2025-11-20")
)]
#[case::tool_search_for_azure(
    "azure",
    r#"{"tools": [{"type": "tool_search_tool_bm25_20251119"}]}"#,
    "[]",
    Some("advanced-tool-use-2025-11-20")
)]
#[case::tool_search_for_azure_ai(
    "azure_ai",
    r#"{"tools": [{"type": "tool_search_tool_regex_20251119"}]}"#,
    "[]",
    Some("advanced-tool-use-2025-11-20")
)]
#[case::tool_search_for_unknown_provider(
    "unknown",
    r#"{"tools": [{"type": "tool_search_tool_bm25_20251119"}]}"#,
    "[]",
    Some("advanced-tool-use-2025-11-20")
)]
#[case::thinking_display_updates(
    "anthropic",
    r#"{"thinking": {"type": "enabled", "display": "updates"}}"#,
    "[]",
    Some("thinking-display-updates-2026-08-18")
)]
#[case::adaptive_thinking_display_updates(
    "anthropic",
    r#"{"thinking": {"type": "adaptive", "display": "updates"}}"#,
    "[]",
    Some("thinking-display-updates-2026-08-18")
)]
#[case::disabled_thinking_display_updates(
    "anthropic",
    r#"{"thinking": {"type": "disabled", "display": "updates"}}"#,
    "[]",
    None
)]
#[case::thinking_without_display("anthropic", r#"{"thinking": {"type": "enabled"}}"#, "[]", None)]
#[case::system_tool_addition(
    "anthropic",
    "{}",
    r#"[{"role": "system", "content": [{"type": "tool_addition", "tool": {"type": "tool_reference"}}]}]"#,
    Some("mid-conversation-tool-changes-2026-07-01")
)]
#[case::system_tool_removal(
    "anthropic",
    "{}",
    r#"[{"role": "system", "content": [{"type": "tool_removal", "tool": {"type": "tool_reference"}}]}]"#,
    Some("mid-conversation-tool-changes-2026-07-01")
)]
#[case::user_tool_addition(
    "anthropic",
    "{}",
    r#"[{"role": "user", "content": [{"type": "tool_addition", "tool": {"type": "tool_reference"}}]}]"#,
    None
)]
fn tool_and_thinking_betas(
    #[case] provider: &str,
    #[case] optional_params: &str,
    #[case] request_messages: &str,
    #[case] expected: Option<&str>,
) {
    let merged = inject(&[], provider, optional_params, request_messages);
    assert_eq!(beta_header(&merged), expected);
}

#[test]
fn provider_mapping_that_is_not_an_object_is_rejected() {
    let broken = matrix(r#"{"anthropic": []}"#);
    let tokens = vec!["effort-2025-11-24".to_owned()];
    let error = filter_beta_tokens(&broken, "anthropic", &tokens).unwrap_err();
    assert_eq!(
        error,
        BetaFilterError::ProviderMapping {
            provider: "anthropic".to_owned()
        }
    );
}

#[test]
fn mapping_value_that_is_neither_string_nor_null_is_rejected() {
    let broken = matrix(r#"{"anthropic": {"effort-2025-11-24": ["nested"]}}"#);
    let tokens = vec!["effort-2025-11-24".to_owned()];
    let error = filter_beta_tokens(&broken, "anthropic", &tokens).unwrap_err();
    assert_eq!(
        error,
        BetaFilterError::MappingValue {
            provider: "anthropic".to_owned(),
            token: "effort-2025-11-24".to_owned()
        }
    );
}

#[rstest]
#[case::body_is_not_a_list(r#"{"anthropic_beta": "effort-2025-11-24"}"#)]
#[case::body_holds_a_non_string(r#"{"anthropic_beta": ["effort-2025-11-24", 3]}"#)]
fn body_beta_field_outside_its_type_is_rejected(#[case] body_json: &str) {
    let mut request_headers = headers(&[]);
    let mut body: IndexMap<String, Json> = params(body_json);
    let error = filter_request_betas(
        &mut request_headers,
        &mut body,
        &matrix(MATRIX),
        "anthropic",
    )
    .unwrap_err();
    assert_eq!(
        error,
        BetaFilterError::Field {
            field: "anthropic_beta",
            expected: "a list of strings"
        }
    );
}

#[rstest]
#[case::context_management(r#"{"context_management": "edits"}"#)]
#[case::context_management_edits(r#"{"context_management": {"edits": null}}"#)]
#[case::context_management_edit(r#"{"context_management": {"edits": ["compact"]}}"#)]
#[case::compaction(r#"{"compaction": "summarize"}"#)]
#[case::output_format(r#"{"output_format": "json_schema"}"#)]
#[case::output_config(r#"{"output_config": "format"}"#)]
#[case::speed(r#"{"speed": 1}"#)]
#[case::thinking(r#"{"thinking": "enabled"}"#)]
#[case::tools(r#"{"tools": {"type": "advisor_20260301"}}"#)]
fn optional_params_outside_their_type_are_rejected(#[case] params_json: &str) {
    let optional_params = params(params_json);
    let error = inject_native_betas(
        headers(&[]),
        &NativeRequest {
            provider: "anthropic",
            optional_params: &optional_params,
            messages: &[],
        },
    )
    .unwrap_err();
    assert!(matches!(error, BetaFilterError::Field { .. }));
}
