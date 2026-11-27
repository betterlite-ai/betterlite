use async_trait::async_trait;

use crate::health_services::service_probes::ServiceTestError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MockMessage {
    pub role: &'static str,
    pub content: &'static str,
}

// the one chat completion every mock branch sends; B8540's adapter forwards these fields to B5656
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MockRequest {
    pub model: &'static str,
    pub messages: [MockMessage; 1],
    pub user: &'static str,
    pub mock_response: &'static str,
}

pub const MOCK_REQUEST: MockRequest = MockRequest {
    model: "openai/litellm-mock-response-model",
    messages: [MockMessage {
        role: "user",
        content: "Hey, how's it going?",
    }],
    user: "litellm:/health/services",
    mock_response: "This is a mock response",
};

#[async_trait]
pub trait MockRequests: Send + Sync {
    async fn send_mock_request(&self, request: &MockRequest) -> Result<(), ServiceTestError>;
}
