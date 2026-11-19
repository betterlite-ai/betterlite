use betterlite_prompts::arguments::json::{Json, Object};
use thiserror::Error;

pub const SUPPORTED_MESSAGE_PARAMS: [&str; 4] = ["role", "content", "attachments", "metadata"];

#[derive(Debug, Error)]
pub enum ParamsError {
    #[error("Azure only accepts content as a string")]
    ContentNotString,
    #[error(
        "Azure doesn't support this attachment. To drop it from the call, set `litellm.drop_params = True."
    )]
    UnsupportedAttachment,
    #[error("attachments should always be a list")]
    AttachmentsNotList,
    #[error("an attachment entry is not an object")]
    MalformedAttachment,
}

impl ParamsError {
    // B184 owns the 400 UnsupportedParamsError; a malformed entry keeps Python's no-status containment failure.
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::ContentNotString | Self::UnsupportedAttachment | Self::AttachmentsNotList => {
                Some(400)
            }
            Self::MalformedAttachment => None,
        }
    }
}

// LiteLLM's mapper for Azure messages; role alone is set before the chain, and unknown keys are dropped (azure.py#L79-L112).
pub fn map_create_message_params(
    non_default: &Object,
    optional: &mut Object,
    drop_params: bool,
) -> Result<(), ParamsError> {
    for (param, value) in non_default {
        if param == "role" {
            optional.insert(param.clone(), value.clone());
        }
        match param.as_str() {
            "metadata" => {
                optional.insert(param.clone(), value.clone());
            }
            "content" => match value {
                Json::Text(text) => {
                    optional.insert(param.clone(), Json::Text(text.clone()));
                }
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                    return Err(ParamsError::ContentNotString);
                }
            },
            "attachments" => map_attachments(value, drop_params)?,
            _ => {}
        }
    }
    Ok(())
}

// The collected file ids are never written back, so a valid attachment list leaves optional_params unchanged (azure.py#L91-L104).
fn map_attachments(value: &Json, drop_params: bool) -> Result<(), ParamsError> {
    let Json::List(items) = value else {
        return Err(ParamsError::AttachmentsNotList);
    };
    for item in items {
        // An item that holds a file_id is indexed, which only a mapping survives; Python's containment test comes first.
        let holds = holds_file_id(item)?;
        if holds && !matches!(item, Json::Object(_)) {
            return Err(ParamsError::MalformedAttachment);
        }
        if !holds && !drop_params {
            return Err(ParamsError::UnsupportedAttachment);
        }
    }
    Ok(())
}

// Python's `"file_id" in item`: a mapping key, a substring of text, a member of a list, and a TypeError for the rest.
fn holds_file_id(item: &Json) -> Result<bool, ParamsError> {
    match item {
        Json::Object(fields) => Ok(fields.contains_key("file_id")),
        Json::Text(text) => Ok(text.contains("file_id")),
        Json::List(inner) => Ok(inner
            .iter()
            .any(|member| matches!(member, Json::Text(text) if text == "file_id"))),
        Json::Null | Json::Bool(_) | Json::Number(_) => Err(ParamsError::MalformedAttachment),
    }
}
