use betterlite_azure::azure_message_attachments::legacy::{ParamsError, map_create_message_params};
use betterlite_logging::analytics::python;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// LiteLLM's default_params: a value equal to None is never selected (utils.py#L24-L28).
pub const STANDARD_MESSAGE_PARAMS: [&str; 4] = ["role", "content", "attachments", "metadata"];

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
    pub provider: String,
    #[serde(default)]
    pub kwargs: Object,
}

impl MessageOptions {
    // The wrapper passes the four named arguments and no extra keywords to the selector (main.py#L885-L891).
    #[must_use]
    pub fn from_data(provider: &str, data: &MessageData) -> Self {
        Self {
            role: Some(Json::Text(data.role.clone())),
            content: Some(data.content.clone()),
            attachments: data.attachments.clone(),
            metadata: data.metadata.clone(),
            provider: provider.to_owned(),
            kwargs: Object::new(),
        }
    }
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
            Self::MalformedAttachment { .. } => None,
        }
    }

    #[must_use]
    pub const fn is_unsupported_params(&self) -> bool {
        !matches!(self, Self::MalformedAttachment { .. })
    }

    // str(exception), which is the prefixed message LiteLLM's UnsupportedParamsError carries.
    #[must_use]
    pub fn python_message(&self) -> String {
        if self.is_unsupported_params() {
            return format!("litellm.UnsupportedParamsError: {self}");
        }
        self.to_string()
    }

    #[must_use]
    pub const fn python_class(&self) -> &'static str {
        if self.is_unsupported_params() {
            "UnsupportedParamsError"
        } else {
            "TypeError"
        }
    }
}

// get_optional_params_add_message: non-null standard values, the provider mapper, then every extra keyword.
pub fn get_optional_params_add_message(
    options: &MessageOptions,
    drop_params: bool,
) -> Result<Object, OptionError> {
    let mut passed = Object::new();
    insert_passed(&mut passed, "role", &options.role);
    insert_passed(&mut passed, "content", &options.content);
    insert_passed(&mut passed, "attachments", &options.attachments);
    insert_passed(&mut passed, "metadata", &options.metadata);
    for (key, value) in &options.kwargs {
        passed.insert(key.clone(), value.clone());
    }
    let mut non_default = Object::new();
    for (key, value) in &passed {
        if STANDARD_MESSAGE_PARAMS.contains(&key.as_str()) && !matches!(value, Json::Null) {
            non_default.insert(key.clone(), value.clone());
        }
    }
    let mut optional = Object::new();
    match options.provider.as_str() {
        "openai" => optional.clone_from(&non_default),
        "azure" => map_create_message_params(&non_default, &mut optional, drop_params)
            .map_err(|error| params_error(error, non_default.get("attachments")))?,
        _ => {}
    }
    for (key, value) in &passed {
        if !STANDARD_MESSAGE_PARAMS.contains(&key.as_str()) {
            optional.insert(key.clone(), value.clone());
        }
    }
    Ok(optional)
}

fn insert_passed(passed: &mut Object, key: &str, value: &Option<Json>) {
    if let Some(value) = value {
        passed.insert(key.to_owned(), value.clone());
    }
}

fn params_error(error: ParamsError, attachments: Option<&Json>) -> OptionError {
    match error {
        ParamsError::ContentNotString => OptionError::ContentNotString,
        ParamsError::UnsupportedAttachment => OptionError::UnsupportedAttachment {
            attachments: attachments.map_or_else(String::new, python_str),
        },
        ParamsError::AttachmentsNotList => OptionError::AttachmentsNotList {
            kind: attachments.map_or_else(|| "<class 'NoneType'>".to_owned(), python_type_name),
            raw: attachments.map_or_else(String::new, python_str),
        },
        ParamsError::MalformedAttachment => OptionError::MalformedAttachment,
    }
}

// the pinned mapper's f-strings render a string bare and every other value through repr
fn python_str(value: &Json) -> String {
    match value {
        Json::Text(text) => text.clone(),
        _ => python_repr(value),
    }
}

// The formatter writes into a Vec, so its io error branch is unreachable.
fn python_repr(value: &Json) -> String {
    python::repr(value).unwrap_or_default()
}

fn python_type(value: &Json) -> &'static str {
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

fn python_type_name(value: &Json) -> String {
    format!("<class '{}'>", python_type(value))
}
