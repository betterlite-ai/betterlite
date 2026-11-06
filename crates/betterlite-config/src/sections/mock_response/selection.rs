use super::{MockResponse, MockToolCall};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MockPermission {
    pub allow_client_mock_response: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct MockPlan<'request> {
    pub response: Option<&'request MockResponse>,
    pub tool_calls: Option<&'request [MockToolCall]>,
    pub choice_count: Option<u32>,
    pub prompt_tokens: Option<u64>,
}

// LiteLLM strips both fields from every request body before routing unless the key or team allows them
#[must_use]
pub fn resolve_mock<'request>(
    operator: Option<&'request MockResponse>,
    permission: MockPermission,
    client_response: Option<&'request MockResponse>,
    client_tool_calls: Option<&'request [MockToolCall]>,
    choice_count: Option<u32>,
) -> Option<MockPlan<'request>> {
    let (response, tool_calls) = if permission.allow_client_mock_response {
        (operator.or(client_response), client_tool_calls)
    } else {
        if client_response.is_some() {
            tracing::warn!(
                field = "mock_response",
                "a client-supplied mock field is stripped; set allow_client_mock_response on the key or team metadata to keep it"
            );
        }
        if client_tool_calls.is_some() {
            tracing::warn!(
                field = "mock_tool_calls",
                "a client-supplied mock field is stripped; set allow_client_mock_response on the key or team metadata to keep it"
            );
        }
        (operator, None)
    };
    if response.is_none() && tool_calls.is_none() {
        return None;
    }
    Some(MockPlan {
        response,
        tool_calls,
        choice_count,
        prompt_tokens: None,
    })
}

mod tests;
