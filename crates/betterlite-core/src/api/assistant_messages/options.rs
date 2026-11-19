use betterlite_azure::azure_message_attachments::legacy::{ParamsError, map_create_message_params};
use betterlite_logging::analytics::python;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// The TypedDict add_message builds from its named arguments; role is not narrowed to a literal here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageData {
    pub role: String,
    pub content: Json,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Json>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Json>,
}

// get_optional_params_add_message's arguments, with the caller's extra keywords kept apart.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageOptions {
    #[serde(default)]
    pub role: Option<Json>,
    #[serde(default)]
    pub content: Option<Json>,
    #[serde(default)]
    pub attachments: Option<Json>,
    #[serde(default)]
    pub metadata: Option<Json>,
    #[serde(rename = "custom_llm_provider", alias = "provider")]
    pub provider: String,
    #[serde(default)]
    pub drop_params: bool,
    #[serde(default)]
    pub kwargs: Object,
}

// The mapper's messages carry the offending value, which B697's error enum does not keep (azure.py#L87-L106).
#[derive(Debug, Error)]
pub enum OptionError {
    #[error("Azure only accepts content as a string.")]
    ContentNotString,
    #[error(
        "Azure doesn't support {attachments}. To drop it from the call, set `litellm.drop_params = True."
    )]
    UnsupportedAttachment { attachments: String },
    #[error(
        "Invalid param. attachments should always be a list. Got={kind}, Expected=List. Raw value={raw}"
    )]
    AttachmentsNotList { kind: String, raw: String },
    // Python's own TypeError for an entry that `"file_id" in item` or `item["file_id"]` rejects
    #[error("an attachment entry is not an object")]
    MalformedAttachment,
}

impl OptionError {
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::ContentNotString
            | Self::UnsupportedAttachment { .. }
            | Self::AttachmentsNotList { .. } => Some(400),
            Self::MalformedAttachment => None,
        }
    }

    // str(exception) with the prefix LiteLLM's own exception classes add
    #[must_use]
    pub fn litellm_message(&self) -> String {
        if matches!(self, Self::MalformedAttachment) {
            return self.to_string();
        }
        format!("litellm.UnsupportedParamsError: {self}")
    }

    #[must_use]
    pub const fn exception_name(&self) -> &'static str {
        match self {
            Self::ContentNotString
            | Self::UnsupportedAttachment { .. }
            | Self::AttachmentsNotList { .. } => "UnsupportedParamsError",
            Self::MalformedAttachment => "TypeError",
        }
    }
}

// get_optional_params_add_message: non-null standard values, the provider mapper, then every extra keyword.
pub fn select_message_params(options: &MessageOptions) -> Result<Object, OptionError> {
    let mut non_default = Object::new();
    for (key, value) in [
        ("role", &options.role),
        ("content", &options.content),
        ("attachments", &options.attachments),
        ("metadata", &options.metadata),
    ] {
        if let Some(present) = value
            && !matches!(present, Json::Null)
        {
            non_default.insert(key.to_owned(), present.clone());
        }
    }
    let mut optional = Object::new();
    match options.provider.as_str() {
        "openai" => optional.clone_from(&non_default),
        "azure" => map_create_message_params(&non_default, &mut optional, options.drop_params)
            .map_err(|error| params_error(&error, non_default.get("attachments")))?,
        _ => {}
    }
    for (key, value) in &options.kwargs {
        optional.insert(key.clone(), value.clone());
    }
    Ok(optional)
}

fn params_error(error: &ParamsError, attachments: Option<&Json>) -> OptionError {
    match error {
        ParamsError::ContentNotString => OptionError::ContentNotString,
        ParamsError::UnsupportedAttachment => OptionError::UnsupportedAttachment {
            attachments: attachments.map_or_else(String::new, rendered_text),
        },
        ParamsError::AttachmentsNotList => OptionError::AttachmentsNotList {
            kind: attachments.map_or_else(
                || "<class 'NoneType'>".to_owned(),
                |value| format!("<class '{}'>", type_name(value)),
            ),
            raw: attachments.map_or_else(String::new, rendered_text),
        },
        ParamsError::MalformedAttachment => OptionError::MalformedAttachment,
    }
}

// the pinned mapper's f-strings render a string bare and every other value through repr
fn rendered_text(value: &Json) -> String {
    match value {
        Json::Text(text) => text.clone(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            python::repr(value).unwrap_or_default()
        }
    }
}

fn type_name(value: &Json) -> &'static str {
    match value {
        Json::Null => "NoneType",
        Json::Bool(_) => "bool",
        Json::Number(number) if number.is_i64() || number.is_u64() => "int",
        Json::Number(_) => "float",
        Json::Text(_) => "str",
        Json::List(_) => "list",
        Json::Object(_) => "dict",
    }
}
