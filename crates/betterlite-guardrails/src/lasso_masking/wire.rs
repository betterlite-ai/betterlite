use std::collections::BTreeMap;

use serde::de::{DeserializeOwned, Error as _};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::bedrock_message_selection::wire::{Content, Role};
use crate::lasso::wire::ToolArguments;

// Box<RawValue> needs serde_json's own deserializer, and flatten always buffers through a generic Content type first, so split known fields from the raw object instead of flattening RawValue.
fn split_fields<Head>(raw: &str) -> serde_json::Result<(Head, BTreeMap<String, Box<RawValue>>)>
where
    Head: DeserializeOwned + Serialize,
{
    let head = serde_json::from_str::<Head>(raw)?;
    let modeled =
        serde_json::from_str::<BTreeMap<String, Box<RawValue>>>(&serde_json::to_string(&head)?)?;
    let mut rest = serde_json::from_str::<BTreeMap<String, Box<RawValue>>>(raw)?;
    rest.retain(|key, _| !modeled.contains_key(key));
    Ok((head, rest))
}

// Each wire struct repeats the same known-fields-plus-extra shape, so one macro emits it from a field list instead of a hand-written copy per struct.
macro_rules! raw_value_record {
    ($name:ident / $head:ident { role: $role_ty:ty, $($field:ident : $ty:ty),* $(,)? } skip) => {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        #[serde(try_from = "Box<RawValue>")]
        pub struct $name {
            pub role: $role_ty,
            $(#[serde(skip_serializing_if = "Option::is_none")] pub $field: $ty,)*
            #[serde(flatten)]
            pub extra: BTreeMap<String, Box<RawValue>>,
        }

        #[derive(Deserialize, Serialize)]
        struct $head {
            role: $role_ty,
            $(#[serde(default)] $field: $ty,)*
        }

        impl TryFrom<Box<RawValue>> for $name {
            type Error = serde_json::Error;

            fn try_from(raw: Box<RawValue>) -> Result<Self, Self::Error> {
                let (head, extra) = split_fields::<$head>(raw.get())?;
                Ok(Self { role: head.role, $($field: head.$field,)* extra })
            }
        }
    };

    ($name:ident / $head:ident { role: $role_ty:ty, $($field:ident : $ty:ty),* $(,)? }) => {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        #[serde(try_from = "Box<RawValue>")]
        pub struct $name {
            pub role: $role_ty,
            $(pub $field: $ty,)*
            #[serde(flatten)]
            pub extra: BTreeMap<String, Box<RawValue>>,
        }

        #[derive(Deserialize, Serialize)]
        struct $head {
            role: $role_ty,
            $(#[serde(default)] $field: $ty,)*
        }

        impl TryFrom<Box<RawValue>> for $name {
            type Error = serde_json::Error;

            fn try_from(raw: Box<RawValue>) -> Result<Self, Self::Error> {
                let (head, extra) = split_fields::<$head>(raw.get())?;
                Ok(Self { role: head.role, $($field: head.$field,)* extra })
            }
        }
    };

    ($name:ident / $head:ident { $($field:ident : $ty:ty),* $(,)? }) => {
        #[derive(Debug, Clone, Default, Serialize, Deserialize)]
        #[serde(try_from = "Box<RawValue>")]
        pub struct $name {
            $(pub $field: $ty,)*
            #[serde(flatten)]
            pub extra: BTreeMap<String, Box<RawValue>>,
        }

        #[derive(Default, Deserialize, Serialize)]
        struct $head {
            $(#[serde(default)] $field: $ty,)*
        }

        impl TryFrom<Box<RawValue>> for $name {
            type Error = serde_json::Error;

            fn try_from(raw: Box<RawValue>) -> Result<Self, Self::Error> {
                let (head, extra) = split_fields::<$head>(raw.get())?;
                Ok(Self { $($field: head.$field,)* extra })
            }
        }
    };
}

raw_value_record!(Message / MessageHead {
    role: Role,
    content: Option<Content>,
    tool_calls: Option<Vec<ToolCall>>,
    tool_call_id: Option<String>,
} skip);

raw_value_record!(ToolCall / ToolCallHead {
    id: Option<String>,
    function: Option<ToolCallFunction>,
});

raw_value_record!(ToolCallFunction / ToolCallFunctionHead {
    name: Option<String>,
    arguments: Option<String>,
});

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LassoMessage {
    pub role: String,
    pub content: LassoContent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
#[serde(try_from = "Box<RawValue>")]
pub enum LassoContent {
    ToolUse(ToolUseBlock),
    ToolResult(ToolResultBlock),
    Text(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "Box<RawValue>")]
pub struct ToolUseBlock {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: Option<String>,
    pub name: Option<String>,
    pub input: Option<ToolArguments>,
}

#[derive(Deserialize)]
struct ToolUseBlockHead {
    #[serde(rename = "type")]
    kind: String,
    id: Option<String>,
    name: Option<String>,
    #[serde(default)]
    input: Option<Box<RawValue>>,
}

impl TryFrom<Box<RawValue>> for ToolUseBlock {
    type Error = serde_json::Error;

    fn try_from(raw: Box<RawValue>) -> Result<Self, Self::Error> {
        let ToolUseBlockHead {
            kind,
            id,
            name,
            input: raw_input,
        } = serde_json::from_str(raw.get())?;
        Ok(Self {
            kind,
            id,
            name,
            input: raw_input.as_deref().map(parse_tool_arguments),
        })
    }
}

fn parse_tool_arguments(value: &RawValue) -> ToolArguments {
    let Ok(parsed) = serde_json::from_str(value.get()) else {
        return ToolArguments::Malformed {
            arguments: value.get().to_owned(),
        };
    };
    ToolArguments::Parsed(parsed)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultBlock {
    #[serde(rename = "type")]
    pub kind: String,
    pub tool_use_id: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
}

#[derive(Deserialize)]
struct ContentTag {
    #[serde(rename = "type")]
    kind: String,
}

impl TryFrom<Box<RawValue>> for LassoContent {
    type Error = serde_json::Error;

    fn try_from(raw: Box<RawValue>) -> Result<Self, Self::Error> {
        if raw.get().starts_with('"') {
            return serde_json::from_str(raw.get()).map(Self::Text);
        }
        let tag = serde_json::from_str::<ContentTag>(raw.get())?;
        match tag.kind.as_str() {
            "tool_use" => serde_json::from_str(raw.get()).map(Self::ToolUse),
            "tool_result" => serde_json::from_str(raw.get()).map(Self::ToolResult),
            other => Err(serde_json::Error::custom(format!(
                "unknown Lasso content type `{other}`"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "Box<RawValue>")]
pub struct ModelResponseShape {
    pub choices: Vec<ResponseChoice>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Box<RawValue>>,
}

#[derive(Deserialize, Serialize)]
struct ModelResponseShapeHead {
    #[serde(default)]
    choices: Vec<ResponseChoice>,
}

impl TryFrom<Box<RawValue>> for ModelResponseShape {
    type Error = serde_json::Error;

    fn try_from(raw: Box<RawValue>) -> Result<Self, Self::Error> {
        let (head, extra) = split_fields::<ModelResponseShapeHead>(raw.get())?;
        Ok(Self {
            choices: head.choices,
            extra,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "Box<RawValue>")]
pub struct ResponseChoice {
    pub message: Option<ResponseMessage>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Box<RawValue>>,
}

#[derive(Deserialize, Serialize)]
struct ResponseChoiceHead {
    #[serde(default)]
    message: Option<ResponseMessage>,
}

impl TryFrom<Box<RawValue>> for ResponseChoice {
    type Error = serde_json::Error;

    fn try_from(raw: Box<RawValue>) -> Result<Self, Self::Error> {
        let (head, extra) = split_fields::<ResponseChoiceHead>(raw.get())?;
        Ok(Self {
            message: head.message,
            extra,
        })
    }
}

raw_value_record!(ResponseMessage / ResponseMessageHead {
    role: Role,
    content: Option<String>,
    tool_calls: Option<Vec<ToolCall>>,
});
