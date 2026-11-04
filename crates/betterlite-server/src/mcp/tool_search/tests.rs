#![cfg(test)]

use super::identity::{
    MCPProxyToolIdentityError, mcp_proxy_tool_id, proxy_schema_result, proxy_search_result,
    with_mcp_proxy_identity,
};
use super::settings::{MCPToolSearchSettings, coerce_top_k};
use super::virtual_tools::{
    MCP_PROXY_TOOL_DEFINITIONS, TextContent, TextToolResult, VIRTUAL_TOOL_DEFINITIONS,
};
use super::{
    MCPToolSearchHit, SemanticToolRanker, keyword_score, rank_mcp_tools, search_mcp_tools,
    split_core_tools, tool_text, top_hits,
};
use crate::mcp::catalog_guard::types::MCPTool;
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_semantic::{Embedder, SemanticTextIndex};
use std::collections::BTreeMap;

enum TestEmbedder {
    Vectors,
    Http,
    Broken,
}

#[async_trait::async_trait]
impl Embedder for TestEmbedder {
    async fn embed(
        &self,
        texts: &[String],
    ) -> Result<Vec<betterlite_semantic::Vector>, betterlite_semantic::EmbedError> {
        match self {
            Self::Http => Err(betterlite_semantic::EmbedError::Http {
                status_code: 429,
                detail: "retry later".to_owned(),
            }),
            Self::Broken => Err(betterlite_semantic::EmbedError::Other("broken".to_owned())),
            Self::Vectors => Ok(texts
                .iter()
                .map(|text| match text.as_str() {
                    "query" | "matched\nquery" => vec![1.0_f64, 0.0_f64],
                    _ => vec![0.0_f64, 1.0_f64],
                })
                .collect()),
        }
    }
}

fn tool(name: &str, description: &str) -> MCPTool {
    MCPTool {
        name: name.to_owned(),
        description: Some(description.to_owned()),
        title: None,
        input_schema: BTreeMap::new(),
        output_schema: None,
        execution: None,
        annotations: None,
        icons: None,
        meta: None,
    }
}

#[tokio::test]
async fn keyword_score_counts_case_insensitive_tokens() {
    let source = tool("weather_forecast", "Get current weather and forecast");
    assert_eq!(
        keyword_score("WEATHER forecast", &source).to_bits(),
        2.0_f64.to_bits()
    );
    assert_eq!(keyword_score("weath", &source).to_bits(), 1.0_f64.to_bits());
    assert_eq!(
        search_mcp_tools(
            "weather",
            &[source],
            5,
            &MCPToolSearchSettings::default(),
            None
        )
        .await
        .unwrap()
        .first()
        .and_then(|hit| hit.score),
        None
    );
}

#[test]
fn core_tools_keep_configured_order_outside_the_rank_limit() {
    let tools = vec![
        tool("one", "shared"),
        tool("two", "shared"),
        tool("three", "shared"),
    ];
    let core_names = vec!["two".to_owned(), "two".to_owned()];
    let (core, rest) = split_core_tools(&tools, &core_names);
    assert_eq!(
        core.iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>(),
        ["two"]
    );
    assert_eq!(
        rest.iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>(),
        ["one", "three"]
    );
    let hits = top_hits(&rest, &[1.0_f64, 1.0_f64], 1.0_f64, 1).unwrap();
    assert_eq!(hits[0].1.name, "one");
}

#[test]
fn identity_round_trips_through_metadata_and_rejects_missing_identity() {
    let original = tool("get_weather", "weather");
    let identified = with_mcp_proxy_identity(&original, "server-1");
    let id = mcp_proxy_tool_id(&identified).unwrap();
    assert_eq!(id, "af851ef443d107b1a0eb10eebf3e00cf");
    assert_eq!(
        mcp_proxy_tool_id(&original).unwrap_err(),
        MCPProxyToolIdentityError::Missing
    );
}

#[test]
fn settings_default_and_validate_bounds() {
    let defaults: MCPToolSearchSettings = serde_json::from_str("{}").unwrap();
    assert_eq!(defaults.embedding_model, None);
    assert_eq!(defaults.top_k, 5);
    assert_eq!(defaults.similarity_threshold.to_bits(), 0.0_f64.to_bits());
    assert!(defaults.core_tools.is_empty());
    let error = serde_json::from_str::<MCPToolSearchSettings>(r#"{"top_k":101}"#).unwrap_err();
    assert!(error.to_string().contains("top_k"));
    let threshold =
        serde_json::from_str::<MCPToolSearchSettings>(r#"{"similarity_threshold":1.1}"#)
            .unwrap_err();
    assert!(threshold.to_string().contains("similarity_threshold"));
}

#[test]
fn proxy_results_and_virtual_tool_definitions_preserve_schemas() {
    let mut source_tool = tool("get_weather", "weather");
    source_tool.output_schema = Some(BTreeMap::new());
    let identified = with_mcp_proxy_identity(&source_tool, "server-1");
    assert_eq!(
        proxy_schema_result(&identified).unwrap().output_schema,
        Some(BTreeMap::new())
    );
    assert_eq!(
        proxy_search_result(&MCPToolSearchHit {
            tool: identified,
            score: Some(0.75_f64)
        })
        .unwrap()
        .score,
        Some(0.75_f64)
    );
    let definitions = VIRTUAL_TOOL_DEFINITIONS.to_vec();
    let proxy_definitions = MCP_PROXY_TOOL_DEFINITIONS.to_vec();
    assert_eq!(
        definitions.iter().map(|item| item.name).collect::<Vec<_>>(),
        [
            "mcp_tool_search",
            "mcp_tool_call",
            "agent_search",
            "skill_search"
        ]
    );
    assert_eq!(
        proxy_definitions
            .iter()
            .map(|item| item.name)
            .collect::<Vec<_>>(),
        ["search_tools", "get_tool_schema", "call_tool"]
    );
    assert_eq!(
        definitions.first().unwrap().input_schema.required,
        &["query"]
    );
}

#[test]
fn text_tool_result_marks_text_content_and_error_state() {
    let result = TextToolResult {
        content: vec![TextContent::Text {
            text: "failed".to_owned(),
        }],
        is_error: true,
    };
    assert_eq!(
        result.content.first().map(|content| match content {
            TextContent::Text { text } => text.as_str(),
        }),
        Some("failed")
    );
    assert!(result.is_error);
}

#[test]
fn top_k_coercion_uses_the_legacy_fallback() {
    assert_eq!(
        coerce_top_k(&serde_json::from_str("\"not-an-integer\"").unwrap(), 5),
        5
    );
    assert_eq!(coerce_top_k(&serde_json::from_str("3").unwrap(), 5), 3);
    assert_eq!(coerce_top_k(&serde_json::from_str("null").unwrap(), 9), 9);
}

#[tokio::test]
async fn semantic_ranking_uses_threshold_and_preserves_stable_hits() {
    let tools = vec![tool("matched", "query"), tool("unmatched", "other")];
    let embedder = TestEmbedder::Vectors;
    let mut index = SemanticTextIndex::default();
    let mut ranker = SemanticToolRanker {
        embed: &embedder,
        embedding_model: "test-model",
        index: &mut index,
    };
    let settings = MCPToolSearchSettings {
        embedding_model: Some("test-model".to_owned()),
        top_k: 5,
        similarity_threshold: 0.5_f64,
        core_tools: Vec::new(),
    };

    let hits = rank_mcp_tools("query", &tools, 5, &settings, Some(&mut ranker))
        .await
        .unwrap();

    assert_eq!(
        hits.iter()
            .map(|hit| hit.tool.name.as_str())
            .collect::<Vec<_>>(),
        ["matched"]
    );
    assert_eq!(hits.first().and_then(|hit| hit.score), Some(1.0_f64));
}

#[tokio::test]
async fn empty_query_returns_core_tools_without_ranking_rest() {
    let tools = vec![tool("core", "always present"), tool("ordinary", "match")];
    let settings = MCPToolSearchSettings {
        embedding_model: None,
        top_k: 1,
        similarity_threshold: 0.0_f64,
        core_tools: vec!["core".to_owned()],
    };

    let hits = rank_mcp_tools("", &tools, 1, &settings, None)
        .await
        .unwrap();

    assert_eq!(hits.len(), 1);
    assert_eq!(hits.first().map(|hit| hit.tool.name.as_str()), Some("core"));
    assert_eq!(hits.first().and_then(|hit| hit.score), None);
}

#[test]
fn schema_projection_keeps_snake_case_identity() {
    let identified = with_mcp_proxy_identity(&tool("get_weather", "weather"), "server-1");
    assert_json(
        &proxy_schema_result(&identified).unwrap(),
        r#"{
        "tool_id":"af851ef443d107b1a0eb10eebf3e00cf", "name":"get_weather", "description":"weather", "inputSchema":{}
    }"#,
    );
}

#[rstest::rstest]
#[case::negative("-2", -2)]
#[case::fraction("2.9", 2)]
#[case::negative_fraction("-2.9", -2)]
#[case::false_flag("false", 0)]
#[case::true_flag("true", 1)]
#[case::null("null", 5)]
#[case::object("{}", 5)]
#[case::fraction_text("\"2.9\"", 5)]
#[case::spaced("\" 3 \"", 3)]
fn coercion_preserves_legacy_integers(#[case] value: &str, #[case] expected: i64) {
    assert_eq!(
        coerce_top_k(&serde_json::from_str(value).unwrap(), 5),
        expected
    );
}

#[rstest::rstest]
#[case::null("null")]
#[case::null_fields(r#"{"top_k":null,"core_tools":null,"similarity_threshold":null}"#)]
fn null_settings_take_defaults(#[case] input: &str) {
    assert_eq!(
        serde_json::from_str::<MCPToolSearchSettings>(input).unwrap(),
        MCPToolSearchSettings::default()
    );
}

#[test]
fn empty_name_does_not_add_a_leading_newline() {
    assert_eq!(tool_text(&tool("", "description")), "description");
}

#[tokio::test]
async fn semantic_http_errors_keep_their_status() {
    let mut index = SemanticTextIndex::default();
    let mut ranker = SemanticToolRanker {
        embed: &TestEmbedder::Http,
        embedding_model: "model",
        index: &mut index,
    };
    let error = search_mcp_tools(
        "query",
        &[tool("matched", "query")],
        5,
        &MCPToolSearchSettings::default(),
        Some(&mut ranker),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(error, super::SearchError::Semantic(betterlite_semantic::SemanticError::Http {status_code: 429, detail}) if detail == "retry later")
    );
}

#[test]
fn all_virtual_literals_match_the_pinned_definitions() {
    let definitions = [
        VIRTUAL_TOOL_DEFINITIONS.to_vec(),
        MCP_PROXY_TOOL_DEFINITIONS.to_vec(),
    ]
    .concat();
    assert_json(
        &definitions,
        include_str!("fixtures/virtual-definitions.json"),
    );
}

#[rstest::rstest]
#[case::missing("{}")]
#[case::wrong_identity(r#"{"litellm.ai/proxy_tool_identity":3}"#)]
#[case::missing_server(r#"{"litellm.ai/proxy_tool_identity":{"tool_name":"x"}}"#)]
#[case::wrong_name(r#"{"litellm.ai/proxy_tool_identity":{"server_id":"s","tool_name":7}}"#)]
fn invalid_identity_is_a_named_error(#[case] metadata: &str) {
    let mut candidate = tool("x", "description");
    candidate.meta = Some(serde_json::from_str(metadata).unwrap());
    let error = mcp_proxy_tool_id(&candidate).unwrap_err();
    assert!(error.to_string().contains("identity"));
}

#[test]
fn identity_keeps_original_name_and_unrelated_metadata() {
    let mut original = tool("get_weather", "weather");
    original.meta = Some(BTreeMap::from([(
        "other".to_owned(),
        JsonValue::Bool(true),
    )]));
    let mut identified = with_mcp_proxy_identity(&original, "server-1");
    identified.name = "renamed".to_owned();
    assert_eq!(
        mcp_proxy_tool_id(&identified).unwrap(),
        "af851ef443d107b1a0eb10eebf3e00cf"
    );
    assert_eq!(
        identified.meta.as_ref().unwrap().get("other"),
        Some(&JsonValue::Bool(true))
    );
    assert_eq!(original.meta.as_ref().unwrap().len(), 1);
    assert_ne!(
        mcp_proxy_tool_id(&with_mcp_proxy_identity(&original, "server-2")).unwrap(),
        mcp_proxy_tool_id(&identified).unwrap()
    );
}

#[rstest::rstest]
#[case::low_top_k(r#"{"top_k":0}"#, "top_k")]
#[case::high_top_k(r#"{"top_k":101}"#, "top_k")]
#[case::low_threshold(r#"{"similarity_threshold":-0.1}"#, "similarity_threshold")]
#[case::high_threshold(r#"{"similarity_threshold":1.1}"#, "similarity_threshold")]
#[case::unknown(r#"{"typo":1}"#, "typo")]
fn settings_reject_invalid_fields(#[case] input: &str, #[case] field: &str) {
    let error = serde_json::from_str::<MCPToolSearchSettings>(input).unwrap_err();
    assert!(error.to_string().contains(field));
}

#[tokio::test]
async fn core_hits_are_outside_limits_and_unscored() {
    let settings = MCPToolSearchSettings {
        embedding_model: None,
        top_k: 1,
        similarity_threshold: 1.0_f64,
        core_tools: vec!["core".to_owned()],
    };
    let tools = [
        tool("a", "query"),
        tool("core", "unrelated"),
        tool("b", "query"),
    ];
    let hits = search_mcp_tools("query", &tools, 9, &settings, None)
        .await
        .unwrap();
    assert_eq!(
        hits.iter().map(|hit| hit.name.as_str()).collect::<Vec<_>>(),
        ["core", "a"]
    );
    assert!(hits.iter().all(|hit| hit.score.is_none()));
    let zero = search_mcp_tools("query", &tools, 0, &settings, None)
        .await
        .unwrap();
    assert_eq!(zero.len(), 1);
}

#[test]
fn core_names_deduplicate_and_select_last_matching_tool() {
    let tools = [
        tool("core", "old"),
        tool("other", "one"),
        tool("core", "new"),
    ];
    let names = ["missing".to_owned(), "core".to_owned(), "core".to_owned()];
    let (core, rest) = split_core_tools(&tools, &names);
    assert_eq!(core, [tool("core", "new")]);
    assert_eq!(rest, [tool("other", "one")]);
}

#[test]
fn top_hits_include_threshold_stabilize_ties_and_reject_bad_lengths() {
    let tools = [
        tool("first", ""),
        tool("second", ""),
        tool("third", ""),
        tool("fourth", ""),
    ];
    let hits = top_hits(&tools, &[0.5_f64, 0.9_f64, 0.9_f64, 0.4_f64], 0.5_f64, 3).unwrap();
    assert_eq!(
        hits.iter()
            .map(|(_, hit)| hit.name.as_str())
            .collect::<Vec<_>>(),
        ["second", "third", "first"]
    );
    let error = top_hits(&tools, &[1.0_f64], 0.5_f64, 2).unwrap_err();
    assert!(matches!(
        error,
        super::SearchError::ScoreCount {
            tools: 4,
            scores: 1
        }
    ));
    assert_eq!(
        top_hits(&tools, &[1.0_f64, 1.0_f64, 1.0_f64, 1.0_f64], 0.0_f64, -2)
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn empty_query_skips_even_a_failing_embedder() {
    let settings = MCPToolSearchSettings {
        embedding_model: Some("model".to_owned()),
        top_k: 1,
        similarity_threshold: 0.0_f64,
        core_tools: vec!["core".to_owned()],
    };
    let mut index = SemanticTextIndex::default();
    let mut ranker = SemanticToolRanker {
        embed: &TestEmbedder::Http,
        embedding_model: "model",
        index: &mut index,
    };
    let hits = search_mcp_tools(
        "",
        &[tool("core", ""), tool("rest", "")],
        0,
        &settings,
        Some(&mut ranker),
    )
    .await
    .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].name, "core");
    assert_eq!(hits[0].score, None);
}

#[rstest::rstest]
#[case::success(
    false,
    r#"{"content":[{"type":"text","text":"response"}],"isError":false}"#
)]
#[case::failure(
    true,
    r#"{"content":[{"type":"text","text":"response"}],"isError":true}"#
)]
fn text_result_has_exact_wire_shape(#[case] is_error: bool, #[case] expected: &str) {
    let result = TextToolResult {
        content: vec![TextContent::Text {
            text: "response".to_owned(),
        }],
        is_error,
    };
    assert_json(&result, expected);
}

#[tokio::test]
async fn embedding_failure_is_preserved_without_keyword_fallback() {
    let mut index = SemanticTextIndex::default();
    let mut ranker = SemanticToolRanker {
        embed: &TestEmbedder::Broken,
        embedding_model: "model",
        index: &mut index,
    };
    let error = search_mcp_tools(
        "query",
        &[tool("query", "")],
        5,
        &MCPToolSearchSettings::default(),
        Some(&mut ranker),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(error, super::SearchError::Semantic(betterlite_semantic::SemanticError::Failed(failure)) if failure.reason == "embedding the search query failed: broken")
    );
}

#[tokio::test]
async fn semantic_projection_includes_zero_score_at_threshold() {
    let mut index = SemanticTextIndex::default();
    let mut ranker = SemanticToolRanker {
        embed: &TestEmbedder::Vectors,
        embedding_model: "model",
        index: &mut index,
    };
    let hits = search_mcp_tools(
        "query",
        &[tool("matched", "query"), tool("other", "")],
        5,
        &MCPToolSearchSettings::default(),
        Some(&mut ranker),
    )
    .await
    .unwrap();
    assert_json(
        &hits,
        r#"[
        {"name":"matched","description":"query","inputSchema":{},"score":1.0},
        {"name":"other","description":"","inputSchema":{},"score":0.0}
    ]"#,
    );
}

#[test]
fn projections_preserve_schemas_and_omit_absent_scores() {
    let mut candidate = tool("x", "");
    candidate.description = None;
    candidate.input_schema =
        serde_json::from_str(r#"{"type":"object","required":["value"]}"#).unwrap();
    candidate.output_schema = Some(serde_json::from_str(r#"{"type":"string"}"#).unwrap());
    let identified = with_mcp_proxy_identity(&candidate, "s");
    assert_json(
        &proxy_schema_result(&identified).unwrap(),
        r#"{
        "tool_id":"49b2c3ba204ce4d4646259d989af5376", "name":"x", "description":"",
        "inputSchema":{"type":"object","required":["value"]}, "outputSchema":{"type":"string"}
    }"#,
    );
    let projection = proxy_search_result(&MCPToolSearchHit {
        tool: identified,
        score: Some(0.0_f64),
    })
    .unwrap();
    assert_json(
        &projection,
        r#"{
        "tool_id":"49b2c3ba204ce4d4646259d989af5376", "name":"x", "description":"", "score":0.0
    }"#,
    );
}

fn assert_json<T>(actual: &T, expected: &str)
where
    T: serde::Serialize,
{
    let actual_json: JsonValue =
        serde_json::from_str(&serde_json::to_string(actual).unwrap()).unwrap();
    let expected_json: JsonValue = serde_json::from_str(expected).unwrap();
    assert_eq!(actual_json, expected_json);
}

#[test]
fn signed_zero_scores_keep_original_order() {
    let tools = [tool("first", ""), tool("second", "")];
    let hits = top_hits(&tools, &[-0.0_f64, 0.0_f64], 0.0, 2).unwrap();
    assert_eq!(
        hits.iter()
            .map(|(_, item)| item.name.as_str())
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
}
