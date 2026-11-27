use betterlite_mcp::client::tools::result::{CallToolResult, ContentBlock};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_server::mcp::search_handlers::error::SearchHandlerError;
use betterlite_server::mcp::search_handlers::types::{
    AgentSearchRequest, HandlerContext, McpToolSearchRequest, ProxyToolRequest, SkillSearchRequest,
    ToolCallRequest,
};
use betterlite_server::mcp::search_handlers::{
    handle_agent_search, handle_mcp_proxy_tool, handle_mcp_tool_call, handle_mcp_tool_search,
    handle_skill_search,
};
use serde_json::Value as JsonValue;
use serde_json::json;
use tokio::runtime::Builder;

use super::fixtures::fakes::Fakes;
use super::input::{DEFAULT_TOP_K, Handler, Input, invalid_input};

type HandlerOutcome = Result<CallToolResult, SearchHandlerError>;

pub(super) fn run(input: &Input) -> HelperResult {
    let fakes = Fakes::new(input)?;
    let runtime = Builder::new_current_thread()
        .build()
        .map_err(|error| invalid_input(error.to_string()))?;
    let result = runtime.block_on(dispatch(input, &fakes))?;
    envelope(&result, &fakes)
}

async fn dispatch(input: &Input, fakes: &Fakes<'_>) -> Result<CallToolResult, Box<HelperError>> {
    let context = HandlerContext {
        caller: &fakes.caller,
        scope: &fakes.scope,
        ports: fakes,
        operations: fakes,
        settings: fakes,
    };
    let outcome = match input.handler {
        Handler::AgentSearch => agent_search(input, &context).await,
        Handler::SkillSearch => skill_search(input, &context).await,
        Handler::McpToolSearch => mcp_tool_search(input, &context).await,
        Handler::McpProxyTool => mcp_proxy_tool(input, &context).await,
        Handler::McpToolCall => mcp_tool_call(input, &context).await,
    };
    outcome.map_err(|error| handler_error(&error))
}

async fn agent_search(input: &Input, context: &HandlerContext<'_>) -> HandlerOutcome {
    let request = AgentSearchRequest {
        query: input.query.as_deref().unwrap_or_default(),
        top_k: input.top_k.unwrap_or(DEFAULT_TOP_K),
    };
    handle_agent_search(request, context).await
}

async fn skill_search(input: &Input, context: &HandlerContext<'_>) -> HandlerOutcome {
    let request = SkillSearchRequest {
        query: input.query.as_deref().unwrap_or_default(),
        top_k: input.top_k.unwrap_or(DEFAULT_TOP_K),
    };
    handle_skill_search(request, context).await
}

async fn mcp_tool_search(input: &Input, context: &HandlerContext<'_>) -> HandlerOutcome {
    let request = McpToolSearchRequest {
        query: input.query.as_deref().unwrap_or_default(),
        top_k: input.top_k.unwrap_or(DEFAULT_TOP_K),
    };
    handle_mcp_tool_search(request, context).await
}

async fn mcp_proxy_tool(input: &Input, context: &HandlerContext<'_>) -> HandlerOutcome {
    let name = input.name.as_deref().unwrap_or_default();
    let request = ProxyToolRequest {
        name,
        arguments: &input.arguments,
    };
    handle_mcp_proxy_tool(request, context).await
}

async fn mcp_tool_call(input: &Input, context: &HandlerContext<'_>) -> HandlerOutcome {
    let request = ToolCallRequest {
        tool_name: input.tool_name.as_deref().unwrap_or_default(),
        arguments: &input.call_arguments,
        requested_server_id: input.requested_server_id.as_deref(),
        guardrail_context: input.guardrail_context.as_ref(),
    };
    handle_mcp_tool_call(request, context).await
}

fn handler_error(error: &SearchHandlerError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "SearchHandlerError".to_owned(),
        status: Some(error.status_code().as_u16()),
        error_type: None,
        param: None,
        code: None,
        message: error.detail(),
    })
}

fn envelope(result: &CallToolResult, fakes: &Fakes<'_>) -> HelperResult {
    let content = result
        .content
        .iter()
        .map(content_value)
        .collect::<Result<Vec<_>, Box<HelperError>>>()?;
    Ok(json!({
        "content": content,
        "isError": result.is_error,
        "observed": fakes.observations(),
    }))
}

fn content_value(block: &ContentBlock) -> Result<JsonValue, Box<HelperError>> {
    match block {
        ContentBlock::Text(text) => Ok(text_block(&text.text)),
        ContentBlock::Image(_image)
        | ContentBlock::Audio(_audio)
        | ContentBlock::ResourceLink(_link)
        | ContentBlock::Resource(_resource) => Err(invalid_input(
            "the search handlers return text content blocks only".to_owned(),
        )),
    }
}

fn text_block(text: &str) -> JsonValue {
    match serde_json::from_str::<JsonValue>(text) {
        Ok(parsed) => json!({"type": "text", "json": parsed}),
        Err(_error) => json!({"type": "text", "text": text}),
    }
}
