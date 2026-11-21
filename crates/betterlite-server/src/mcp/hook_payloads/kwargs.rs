use betterlite_models::decisions::HiddenParams;
use serde::Serialize;

use crate::mcp::hook_payloads::{
    GuardrailHookData, McpHookKwargs, McpPreCallRequest, McpPreCallResponse, ToolArguments,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PreCallDecision {
    pub should_proceed: bool,
    pub modified_arguments: ToolArguments,
    pub error_message: Option<String>,
    pub hidden_params: HiddenParams,
}

#[must_use]
pub fn parse_pre_mcp_call_response(
    response: McpPreCallResponse,
    original: &McpPreCallRequest,
) -> PreCallDecision {
    PreCallDecision {
        should_proceed: response.should_proceed,
        modified_arguments: response
            .modified_arguments
            .unwrap_or_else(|| original.arguments.clone()),
        error_message: response.error_message,
        hidden_params: response.hidden_params,
    }
}

#[must_use]
pub fn mcp_request_from_kwargs(kwargs: &McpHookKwargs) -> McpPreCallRequest {
    McpPreCallRequest {
        tool_name: kwargs.name.clone(),
        arguments: kwargs.arguments.clone(),
        server_name: kwargs.server_name.clone(),
        user_api_key_auth: kwargs.user_api_key_auth.clone().unwrap_or_default(),
        hidden_params: HiddenParams::new(),
    }
}

// a later guardrail's headers win over an earlier one's, since the most specific one (the JWT signer) runs last
#[must_use]
pub fn mcp_response_to_kwargs(
    response: Option<&GuardrailHookData>,
    mut kwargs: McpHookKwargs,
) -> McpHookKwargs {
    let Some(data) = response else {
        return kwargs;
    };
    if let Some(arguments) = &data.modified_arguments {
        kwargs.arguments.clone_from(arguments);
    }
    if let Some(headers) = &data.extra_headers {
        let mut merged = kwargs.extra_headers.take().unwrap_or_default();
        merged.extend(
            headers
                .iter()
                .map(|(name, value)| (name.clone(), value.clone())),
        );
        kwargs.extra_headers = Some(merged);
    }
    kwargs
}
