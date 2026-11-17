use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::bedrock_settings::{BedrockError, ErrorHeaders, ErrorInput, HeaderError};

pub mod environment;
pub mod message;
pub mod request;
mod tests;
pub mod transform;
pub mod wire;

// A Converse reply body, kept as the parsed mapping LiteLLM hands to ConverseResponseBlock; the members the conversion reads are reached through the accessors below.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConverseReply {
    pub(crate) body: IndexMap<String, Json>,
}

// Only reading the body fails the 422 catch in LiteLLM: later missing members keep their own KeyError.
#[derive(Debug, thiserror::Error)]
pub enum ConversionError {
    #[error(
        "Error converting to valid response block=the body is not a JSON object. File an issue if litellm error - https://github.com/BerriAI/litellm/issues"
    )]
    NotAnObject,
    #[error(
        "Error converting to valid response block={source}. File an issue if litellm error - https://github.com/BerriAI/litellm/issues"
    )]
    Json {
        #[source]
        source: serde_json::Error,
    },
}

impl ConversionError {
    pub fn into_bedrock_error(
        self,
        headers: &[(String, String)],
    ) -> Result<BedrockError, HeaderError> {
        // The shared BedrockError keeps HTTP error headers as ASCII text; refuse one it could not carry.
        for (name, value) in headers {
            if !name.is_ascii() || !value.is_ascii() {
                return Err(HeaderError::NotAscii { name: name.clone() });
            }
        }
        BedrockError::try_from(ErrorInput {
            status_code: 422,
            message: self.to_string(),
            request: None,
            body: None,
            status_code_is_synthesized: false,
            headers: Some(ErrorHeaders::Http(headers.to_vec())),
            response: None,
        })
    }
}

// Python records a missing member as a KeyError and a wrongly typed one as a TypeError; both carry no status.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ReplyFieldError {
    #[error("the Converse reply has no {path}")]
    Missing { path: &'static str },
    #[error("the Converse reply member {path} is not {expected}")]
    NotA {
        path: &'static str,
        expected: &'static str,
    },
}

pub fn decode_converse_reply(body: &[u8]) -> Result<ConverseReply, ConversionError> {
    match serde_json::from_slice::<Json>(body) {
        Ok(Json::Object(fields)) => Ok(ConverseReply { body: fields }),
        Ok(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(ConversionError::NotAnObject)
        }
        Err(source) => Err(ConversionError::Json { source }),
    }
}

impl ConverseReply {
    // completion_response["output"]["message"]: output must exist and be a mapping, message must be a key, and null is the legal "no message" value.
    pub fn message(&self) -> Result<Option<&Json>, ReplyFieldError> {
        let Some(output) = self.body.get("output") else {
            return Err(ReplyFieldError::Missing { path: "output" });
        };
        let Json::Object(fields) = output else {
            return Err(ReplyFieldError::NotA {
                path: "output",
                expected: "an object",
            });
        };
        let Some(message) = fields.get("message") else {
            return Err(ReplyFieldError::Missing {
                path: "output.message",
            });
        };
        Ok(match message {
            Json::Null => None,
            other @ (Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_)) => Some(other),
        })
    }

    pub fn usage(&self) -> Result<&Json, ReplyFieldError> {
        self.body
            .get("usage")
            .ok_or(ReplyFieldError::Missing { path: "usage" })
    }

    pub fn stop_reason(&self) -> Result<&Json, ReplyFieldError> {
        self.body
            .get("stopReason")
            .ok_or(ReplyFieldError::Missing { path: "stopReason" })
    }
}
