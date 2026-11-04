#![cfg(test)]

use super::*;

struct StaticEmbedder;

#[async_trait::async_trait]
impl Embedder for StaticEmbedder {
    async fn embed(
        &self,
        texts: &[String],
    ) -> Result<Vec<betterlite_semantic::Vector>, betterlite_semantic::EmbedError> {
        Ok(texts
            .iter()
            .map(|text| match text.as_str() {
                "query" | "matched\nquery" => vec![1.0, 0.0],
                _ => vec![0.0, 1.0],
            })
            .collect())
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

#[test]
fn keyword_score_counts_case_insensitive_tokens() {
    let tool = tool("weather_forecast", "Get current weather and forecast");
    assert_eq!(keyword_score("WEATHER forecast", &tool), 2.0);
    assert_eq!(keyword_score("weath", &tool), 1.0);
    assert_eq!(
        search_tools("weather", &[tool], 5)
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
    let hits = top_hits(&rest, &[1.0, 1.0], 1.0, 1);
    assert_eq!(hits[0].1.name, "one");
}

#[test]
fn identity_round_trips_through_metadata_and_rejects_missing_identity() {
    let original = tool("get_weather", "weather");
    let identified = with_mcp_proxy_identity(&original, "server-1");
    let id = mcp_proxy_tool_id(&identified).unwrap();
    assert_eq!(id.len(), 32);
    assert_eq!(
        mcp_proxy_tool_id(&original).unwrap_err(),
        MCPProxyToolIdentityError::Missing
    );
}

#[test]
fn settings_default_and_validate_bounds() {
    let defaults = MCPToolSearchSettings::from_value(&serde_json::json!({})).unwrap();
    assert_eq!(defaults.embedding_model, None);
    assert_eq!(defaults.top_k, 5);
    assert_eq!(defaults.similarity_threshold, 0.0);
    assert!(defaults.core_tools.is_empty());
    assert!(MCPToolSearchSettings::from_value(&serde_json::json!({"top_k":101})).is_err());
    assert!(
        MCPToolSearchSettings::from_value(&serde_json::json!({"similarity_threshold":1.1}))
            .is_err()
    );
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
            score: Some(0.75)
        })
        .unwrap()
        .score,
        Some(0.75)
    );
    let definitions = get_virtual_tool_definitions();
    let proxy_definitions = get_mcp_proxy_tool_definitions();
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
        definitions
            .first()
            .and_then(|item| item.input_schema.get("required")),
        Some(&serde_json::json!(["query"]))
    );
}

#[test]
fn text_tool_result_marks_text_content_and_error_state() {
    let result = text_tool_result("failed", true);
    assert_eq!(
        result.content.first().map(|content| content.text.as_str()),
        Some("failed")
    );
    assert!(result.is_error);
}

#[test]
fn top_k_coercion_uses_the_legacy_fallback() {
    assert_eq!(coerce_top_k(&serde_json::json!("not-an-integer"), 5), 5);
    assert_eq!(coerce_top_k(&serde_json::json!(3), 5), 3);
}

#[tokio::test]
async fn semantic_ranking_uses_threshold_and_preserves_stable_hits() {
    let tools = vec![tool("matched", "query"), tool("unmatched", "other")];
    let embedder = StaticEmbedder;
    let mut index = SemanticTextIndex::default();
    let mut ranker = SemanticToolRanker {
        embed: &embedder,
        embedding_model: "test-model",
        index: &mut index,
    };
    let settings = MCPToolSearchSettings {
        embedding_model: Some("test-model".to_owned()),
        top_k: 5,
        similarity_threshold: 0.5,
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
    assert_eq!(hits.first().and_then(|hit| hit.score), Some(1.0));
}
