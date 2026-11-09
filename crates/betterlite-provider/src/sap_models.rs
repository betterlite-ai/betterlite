pub mod content;
pub mod filtering;
pub mod grounding;
pub mod masking;
pub mod messages;
pub mod presence;
pub mod request;
pub mod response_format;
pub mod template;
pub mod translation;

mod tests;

use betterlite_prompts::arguments::json::Json;
use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

use self::masking::MaskingModuleConfig;
use self::messages::ChatCompletionTool;
use self::messages::SAPAssistantMessage;
use self::messages::SAPMessage;
use self::messages::SAPToolChatMessage;
use self::messages::SAPUserMessage;
use self::request::OrchestrationRequest;
use self::response_format::ResponseFormat;
use self::response_format::ResponseFormatChoice;
use self::response_format::ResponseFormatJSONSchema;

#[derive(Debug, Error)]
pub enum SapModelError {
    #[error("{path}: {source}")]
    Invalid {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    // Raised inside a model's TryFrom; serde puts its message on the path of the field it was reading.
    #[error("{message}")]
    Rejected { message: String },
    #[error("the validated SAP model could not be written as JSON")]
    Serialize(#[source] serde_json::Error),
}

fn beneath(root: &str, path: &serde_path_to_error::Path) -> String {
    let text = path.to_string();
    if text == "." {
        root.to_owned()
    } else {
        format!("{root}.{text}")
    }
}

fn decode<T>(text: &str, root: &str) -> Result<T, SapModelError>
where
    T: DeserializeOwned,
{
    let mut deserializer = serde_json::Deserializer::from_str(text);
    serde_path_to_error::deserialize::<_, T>(&mut deserializer).map_err(|source| {
        let path = beneath(root, source.path());
        SapModelError::Invalid {
            path,
            source: source.into_inner(),
        }
    })
}

// Serializing the typed model and reading it back keeps the declared field order that Json's insertion-ordered map preserves.
fn encode<T>(model: &T, root: &str) -> Result<Json, SapModelError>
where
    T: Serialize,
{
    let text = serde_json::to_string(model).map_err(SapModelError::Serialize)?;
    serde_json::from_str(&text).map_err(|source| SapModelError::Invalid {
        path: root.to_owned(),
        source,
    })
}

// A union variant's own parse: serde's untagged enums drop the variant's error, so the declared-order unions call this and keep it.
fn parse_nested<T>(body: &Json) -> Result<T, SapModelError>
where
    T: DeserializeOwned,
{
    let text = serde_json::to_string(body).map_err(SapModelError::Serialize)?;
    let mut deserializer = serde_json::Deserializer::from_str(&text);
    match serde_path_to_error::deserialize::<_, T>(&mut deserializer) {
        Ok(model) => Ok(model),
        Err(source) => {
            let path = source.path().to_string();
            let message = source.into_inner().to_string();
            Err(SapModelError::Rejected {
                message: if path == "." {
                    message
                } else {
                    format!("{path}: {message}")
                },
            })
        }
    }
}

fn role(body: &Json) -> Option<&str> {
    match body.as_object().and_then(|object| object.get("role")) {
        Some(Json::Text(name)) => Some(name.as_str()),
        _ => None,
    }
}

fn dump_message<T>(body: &Json) -> Result<Json, SapModelError>
where
    T: DeserializeOwned + Serialize,
{
    let text = serde_json::to_string(body).map_err(SapModelError::Serialize)?;
    let message = decode::<T>(&text, "message")?;
    encode(&message, "message")
}

pub fn validate_orchestration_request(body: &Json) -> Result<Json, SapModelError> {
    let text = serde_json::to_string(body).map_err(SapModelError::Serialize)?;
    let request = decode::<OrchestrationRequest>(&text, "request")?;
    encode(&request, "request")
}

pub fn validate_sap_message(body: &Json) -> Result<Json, SapModelError> {
    match role(body) {
        Some("user") => dump_message::<SAPUserMessage>(body),
        Some("assistant") => dump_message::<SAPAssistantMessage>(body),
        Some("tool") => dump_message::<SAPToolChatMessage>(body),
        _ => dump_message::<SAPMessage>(body),
    }
}

pub fn validate_chat_completion_tool(body: &Json) -> Result<Json, SapModelError> {
    let text = serde_json::to_string(body).map_err(SapModelError::Serialize)?;
    let tool = decode::<ChatCompletionTool>(&text, "tool")?;
    encode(&tool.full(), "tool")
}

fn wants_json_schema(body: &Json) -> bool {
    match body.as_object().and_then(|object| object.get("type")) {
        Some(Json::Text(kind)) => kind == "json_schema",
        _ => false,
    }
}

// LiteLLM's transform sends a json_schema type through ResponseFormatJSONSchema and every other type through ResponseFormat, which is the same split the declared-order union makes.
pub fn validate_response_format(body: &Json) -> Result<Json, SapModelError> {
    let text = serde_json::to_string(body).map_err(SapModelError::Serialize)?;
    let format = if wants_json_schema(body) {
        ResponseFormatChoice::JsonSchema(decode::<ResponseFormatJSONSchema>(
            &text,
            "response_format",
        )?)
    } else {
        ResponseFormatChoice::Plain(decode::<ResponseFormat>(&text, "response_format")?)
    };
    encode(&format, "response_format")
}

pub fn validate_masking_module(body: &Json) -> Result<Json, SapModelError> {
    let text = serde_json::to_string(body).map_err(SapModelError::Serialize)?;
    let module = decode::<MaskingModuleConfig>(&text, "masking_module")?;
    encode(&module, "masking_module")
}
