use serde::Deserializer as _;
use serde::de::{self, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::value::RawValue;
use std::fmt;
use uuid::Uuid;

enum Extracted {
    Mapping(Option<Box<RawValue>>),
    NotMapping(&'static str),
}

struct ModelExtractor;

impl<'de> Visitor<'de> for ModelExtractor {
    type Value = Extracted;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Extracted::NotMapping("NoneType"))
    }

    fn visit_bool<E>(self, _value: bool) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Extracted::NotMapping("bool"))
    }

    fn visit_i64<E>(self, _value: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Extracted::NotMapping("int"))
    }

    fn visit_u64<E>(self, _value: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Extracted::NotMapping("int"))
    }

    fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Extracted::NotMapping("float"))
    }

    fn visit_str<E>(self, _value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Extracted::NotMapping("str"))
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(Extracted::NotMapping("list"))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut model = None;
        while let Some(key) = map.next_key::<String>()? {
            if key == "model" {
                model = Some(map.next_value::<Box<RawValue>>()?);
            } else {
                let _ignored: IgnoredAny = map.next_value()?;
            }
        }
        Ok(Extracted::Mapping(model))
    }
}

#[derive(Debug, Clone)]
pub enum MockModel {
    Default,
    Raw(Box<RawValue>),
}

impl serde::Serialize for MockModel {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Default => serializer.serialize_str("mock-model"),
            Self::Raw(raw) => raw.serialize(serializer),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DecodedMockRequest {
    pub model: MockModel,
    pub stream: bool,
}

impl DecodedMockRequest {
    const fn fallback() -> Self {
        Self {
            model: MockModel::Default,
            stream: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeMockRequestError {
    #[error("'{type_name}' object has no attribute 'get'")]
    NotAMapping { type_name: String },
}

// Any decode failure (bad UTF-8, invalid JSON, trailing bytes) falls back rather than erroring, matching _parse_request.
pub fn decode_mock_request(content: &[u8]) -> Result<DecodedMockRequest, DecodeMockRequestError> {
    let mut deserializer = serde_json::Deserializer::from_slice(content);
    let outcome = deserializer
        .deserialize_any(ModelExtractor)
        .and_then(|extracted| deserializer.end().map(|()| extracted));
    let Ok(extracted) = outcome else {
        return Ok(DecodedMockRequest::fallback());
    };
    match extracted {
        Extracted::Mapping(Some(raw)) => Ok(DecodedMockRequest {
            model: MockModel::Raw(raw),
            stream: false,
        }),
        Extracted::Mapping(None) => Ok(DecodedMockRequest::fallback()),
        Extracted::NotMapping(type_name) => Err(DecodeMockRequestError::NotAMapping {
            type_name: type_name.to_owned(),
        }),
    }
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

pub fn mock_chat_completion<Clock, Identifier>(
    model: MockModel,
    clock: &Clock,
    identifier: &Identifier,
) -> MockChatCompletion
where
    Clock: Fn() -> i64,
    Identifier: Fn() -> Uuid,
{
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
pub fn handle_mock_request<Clock, Identifier>(
    content: &[u8],
    clock: &Clock,
    identifier: &Identifier,
) -> Result<MockHttpResponse, DecodeMockRequestError>
where
    Clock: Fn() -> i64,
    Identifier: Fn() -> Uuid,
{
    let decoded = decode_mock_request(content)?;
    Ok(MockHttpResponse {
        status: 200,
        content_type: "application/json".to_owned(),
        body: mock_chat_completion(decoded.model, clock, identifier),
    })
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
