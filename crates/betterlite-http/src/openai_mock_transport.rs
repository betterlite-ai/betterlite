use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::value::RawValue;
use uuid::Uuid;

// A dict missing "model" fails here and falls to the same mock-model fallback as any other parse failure.
#[derive(Deserialize)]
struct MappingBody {
    model: Box<RawValue>,
}

// Runs only once the mapping attempt fails, so it never has to carry a RawValue through untagged buffering.
#[derive(Deserialize)]
#[serde(untagged)]
enum NotMapping {
    Null,
    Bool(bool),
    UInt(u64),
    Int(i64),
    Float(f64),
    Str(String),
    List(Vec<IgnoredAny>),
}

const fn not_mapping_type_name(value: &NotMapping) -> &'static str {
    match value {
        NotMapping::Null => "NoneType",
        NotMapping::Bool(_flag) => "bool",
        NotMapping::UInt(_magnitude) => "int",
        NotMapping::Int(_magnitude) => "int",
        NotMapping::Float(_magnitude) => "float",
        NotMapping::Str(_text) => "str",
        NotMapping::List(_items) => "list",
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(untagged)]
pub enum MockModel {
    Default(&'static str),
    Raw(Box<RawValue>),
}

#[derive(Debug, Clone)]
pub struct DecodedMockRequest {
    pub model: MockModel,
    pub stream: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeMockRequestError {
    #[error("'{type_name}' object has no attribute 'get'")]
    NotAMapping { type_name: String },
}

// Any decode failure (bad UTF-8, invalid JSON, trailing bytes) falls back rather than erroring, matching _parse_request.
pub fn decode_mock_request(content: &[u8]) -> Result<DecodedMockRequest, DecodeMockRequestError> {
    let mut mapping_deserializer = serde_json::Deserializer::from_slice(content);
    let mapping_outcome = MappingBody::deserialize(&mut mapping_deserializer)
        .and_then(|body| mapping_deserializer.end().map(|()| body));
    if let Ok(body) = mapping_outcome {
        return Ok(DecodedMockRequest {
            model: MockModel::Raw(body.model),
            stream: false,
        });
    }
    let mut scalar_deserializer = serde_json::Deserializer::from_slice(content);
    let scalar_outcome = NotMapping::deserialize(&mut scalar_deserializer)
        .and_then(|value| scalar_deserializer.end().map(|()| value));
    scalar_outcome.map_or_else(
        |_error| {
            Ok(DecodedMockRequest {
                model: MockModel::Default("mock-model"),
                stream: false,
            })
        },
        |value| {
            Err(DecodeMockRequestError::NotAMapping {
                type_name: not_mapping_type_name(&value).to_owned(),
            })
        },
    )
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MockMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MockChoice {
    pub index: u32,
    pub message: MockMessage,
    pub finish_reason: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MockUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MockChatCompletion {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: MockModel,
    pub choices: Vec<MockChoice>,
    pub usage: MockUsage,
}

pub fn mock_chat_completion(
    model: MockModel,
    clock: &dyn Fn() -> i64,
    identifier: &dyn Fn() -> Uuid,
) -> MockChatCompletion {
    let short_id = identifier()
        .simple()
        .to_string()
        .chars()
        .take(8)
        .collect::<String>();
    MockChatCompletion {
        id: format!("chatcmpl-mock-{short_id}"),
        object: "chat.completion".to_owned(),
        created: clock(),
        model,
        choices: vec![MockChoice {
            index: 0,
            message: MockMessage {
                role: "assistant".to_owned(),
                content: "Mock response".to_owned(),
            },
            finish_reason: "stop".to_owned(),
        }],
        usage: MockUsage {
            prompt_tokens: 1,
            completion_tokens: 1,
            total_tokens: 2,
        },
    }
}

#[derive(Debug, Clone)]
pub struct MockHttpResponse {
    pub status: u16,
    pub content_type: String,
    pub body: MockChatCompletion,
}

// handle_async_request and handle_request share this implementation: the mock never performs IO either way.
pub fn handle_mock_request(
    content: &[u8],
    clock: &dyn Fn() -> i64,
    identifier: &dyn Fn() -> Uuid,
) -> Result<MockHttpResponse, DecodeMockRequestError> {
    let decoded = decode_mock_request(content)?;
    let body = mock_chat_completion(decoded.model, clock, identifier);
    let response = MockHttpResponse {
        status: 200,
        content_type: "application/json".to_owned(),
        body,
    };
    Ok(response)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportChoice<Session> {
    Supplied(Session),
    Mock,
    Default,
}

pub fn select_transport<Session, Shared>(
    client_session: Option<Session>,
    network_mock: bool,
    _shared_session: Option<Shared>,
) -> TransportChoice<Session> {
    match client_session {
        Some(session) => TransportChoice::Supplied(session),
        None if network_mock => TransportChoice::Mock,
        None => TransportChoice::Default,
    }
}

#[path = "openai_mock_transport/tests.rs"]
mod tests;
