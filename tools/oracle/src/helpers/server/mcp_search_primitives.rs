use betterlite_server::mcp::catalog_guard::types::MCPTool;
use betterlite_server::mcp::tool_search::identity::{
    MCPProxySchemaResult, MCPProxySearchResult, proxy_schema_result, proxy_search_result,
    with_mcp_proxy_identity,
};
use betterlite_server::mcp::tool_search::settings::MCPToolSearchSettings;
use betterlite_server::mcp::tool_search::virtual_tools::{
    MCP_PROXY_TOOL_DEFINITIONS, VIRTUAL_TOOL_DEFINITIONS,
};
use betterlite_server::mcp::tool_search::{
    MCPToolSearchHit, search_mcp_tools, split_core_tools, top_hits,
};
use serde::{Deserialize, Serialize};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Keyword {
        query: String,
        tools: Vec<MCPTool>,
        top_k: i64,
    },
    Identity {
        tool: Box<MCPTool>,
        server_id: String,
    },
    Core {
        tools: Vec<MCPTool>,
        core_tools: Vec<String>,
    },
    Empty {
        tools: Vec<MCPTool>,
    },
    Settings {
        config: MCPToolSearchSettings,
    },
    Definitions,
    Semantic {
        tools: Vec<MCPTool>,
        scores: Vec<f64>,
        threshold: f64,
        top_k: i64,
    },
}

#[derive(Serialize)]
struct CoreNames<'tools> {
    core: Vec<&'tools str>,
    rest: Vec<&'tools str>,
}

#[derive(Serialize)]
struct IdentityOutput {
    search: MCPProxySearchResult,
    schema: MCPProxySchemaResult,
}

#[derive(Serialize)]
struct ScoredName {
    name: String,
    score: f64,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let output = match input {
            Input::Keyword {
                query,
                tools,
                top_k,
            } => return keyword_results(query, tools, *top_k),
            Input::Identity { tool, server_id } => return identity(tool, server_id),
            Input::Core { tools, core_tools } => return core_names(tools, core_tools),
            Input::Empty { tools } => return keyword_results("", tools, 5),
            Input::Settings { config } => serde_json::to_value(config),
            Input::Definitions => serde_json::to_value(
                [
                    VIRTUAL_TOOL_DEFINITIONS.to_vec(),
                    MCP_PROXY_TOOL_DEFINITIONS.to_vec(),
                ]
                .concat(),
            ),
            Input::Semantic {
                tools,
                scores,
                threshold,
                top_k,
            } => return ranked(tools, scores, *threshold, *top_k),
        };
        output.map_err(helper_error)
    }
}

fn helper_error<E>(error: E) -> Box<crate::helpers::HelperError>
where
    E: std::fmt::Display,
{
    Box::new(crate::helpers::HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn identity(tool: &MCPTool, server_id: &str) -> crate::helpers::HelperResult {
    let identified = with_mcp_proxy_identity(tool, server_id);
    let output = IdentityOutput {
        schema: proxy_schema_result(&identified).map_err(helper_error)?,
        search: proxy_search_result(&MCPToolSearchHit {
            tool: identified,
            score: None,
        })
        .map_err(helper_error)?,
    };
    serde_json::to_value(output).map_err(helper_error)
}

fn core_names(tools: &[MCPTool], names: &[String]) -> crate::helpers::HelperResult {
    let (core, rest) = split_core_tools(tools, names);
    serde_json::to_value(CoreNames {
        core: core.iter().map(|tool| tool.name.as_str()).collect(),
        rest: rest.iter().map(|tool| tool.name.as_str()).collect(),
    })
    .map_err(helper_error)
}

fn ranked(
    tools: &[MCPTool],
    scores: &[f64],
    threshold: f64,
    top_k: i64,
) -> crate::helpers::HelperResult {
    let hits = top_hits(tools, scores, threshold, top_k).map_err(helper_error)?;
    let names: Vec<ScoredName> = hits
        .into_iter()
        .map(|(score, tool)| ScoredName {
            name: tool.name,
            score,
        })
        .collect();
    serde_json::to_value(names).map_err(helper_error)
}

fn keyword_results(query: &str, tools: &[MCPTool], top_k: i64) -> crate::helpers::HelperResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(helper_error)?;
    let settings = MCPToolSearchSettings::default();
    let results = runtime
        .block_on(search_mcp_tools(query, tools, top_k, &settings, None))
        .map_err(helper_error)?;
    serde_json::to_value(results).map_err(helper_error)
}
