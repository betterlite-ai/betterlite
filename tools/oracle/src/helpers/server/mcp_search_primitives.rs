pub(crate) struct Op;

use betterlite_server::mcp::catalog_guard::types::MCPTool;

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        use betterlite_server::mcp::tool_search::{
            MCPToolSearchHit, MCPToolSearchSettings, mcp_proxy_tool_id, search_tools,
            split_core_tools, top_hits, with_mcp_proxy_identity,
        };

        let operation = input
            .get("mode")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let output = match operation {
            "keyword" => {
                let tools: Vec<MCPTool> = serde_json::from_value(
                    input.get("tools").cloned().unwrap_or_default(),
                )
                .map_err(helper_error)?;
                let query = input.get("query").and_then(serde_json::Value::as_str).unwrap_or_default();
                let top_k = input.get("top_k").and_then(serde_json::Value::as_u64)
                    .and_then(|value| usize::try_from(value).ok()).unwrap_or(5);
                serde_json::to_value(search_tools(query, &tools, top_k))
            }
            "identity" => {
                let tool: MCPTool = serde_json::from_value(
                    input.get("tool").cloned().unwrap_or_default(),
                )
                .map_err(helper_error)?;
                let server_id = input.get("server_id").and_then(serde_json::Value::as_str).unwrap_or_default();
                let identified = with_mcp_proxy_identity(&tool, server_id);
                serde_json::to_value(mcp_proxy_tool_id(&identified).map_err(helper_error)?)
            }
            "core" => {
                let tools: Vec<MCPTool> = serde_json::from_value(
                    input.get("tools").cloned().unwrap_or_default(),
                )
                .map_err(helper_error)?;
                let core_names: Vec<String> = serde_json::from_value(input.get("core_tools").cloned().unwrap_or_default())
                    .map_err(helper_error)?;
                let (core, rest) = split_core_tools(&tools, &core_names);
                Ok(serde_json::json!({"core": core.iter().map(|tool| &tool.name).collect::<Vec<_>>(), "rest": rest.iter().map(|tool| &tool.name).collect::<Vec<_>>() }))
            }
            "empty" => Ok(serde_json::json!([])),
            "settings" => {
                let config = input.get("config").cloned().unwrap_or_default();
                let settings = MCPToolSearchSettings::from_value(&config).map_err(helper_error)?;
                serde_json::to_value(settings)
            }
            "semantic" => {
                let tools: Vec<MCPTool> = serde_json::from_value(
                    input.get("tools").cloned().unwrap_or_default(),
                )
                .map_err(helper_error)?;
                let scores: Vec<f64> = serde_json::from_value(input.get("scores").cloned().unwrap_or_default())
                    .map_err(helper_error)?;
                let threshold = input.get("threshold").and_then(serde_json::Value::as_f64).unwrap_or(0.0);
                let limit = input.get("top_k").and_then(serde_json::Value::as_u64)
                    .and_then(|value| usize::try_from(value).ok()).unwrap_or(5);
                serde_json::to_value(top_hits(&tools, &scores, threshold, limit).iter()
                    .map(|(score, tool)| MCPToolSearchHit { tool: tool.clone(), score: Some(*score) })
                    .map(|hit| serde_json::json!({"name":hit.tool.name,"score":hit.score})).collect::<Vec<_>>())
            }
            _ => return Self::not_built("mcp_search_primitives"),
        }.map_err(helper_error)?;
        Ok(output)
    }
}

fn helper_error<E: std::fmt::Display>(error: E) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
