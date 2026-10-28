use crate::bedrock_records::Extensions;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

// Missing fields and explicit nulls must survive positional write-back unchanged.
macro_rules! message_record {
    ($name:ident { $($field:ident: $kind:ty => $key:literal),* $(,)? }) => {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        #[serde(try_from = "Extensions")]
        pub struct $name {
            $(#[serde(rename = $key, skip_serializing_if = "Option::is_none")]
            pub $field: Option<$kind>,)*
            #[serde(flatten)]
            pub extra: Extensions,
        }
        impl TryFrom<Extensions> for $name {
            type Error = serde_json::Error;
            fn try_from(mut extra: Extensions) -> Result<Self, Self::Error> {
                $(let $field = match extra.get($key) {
                    Some(raw) if raw.get() != "null" => {
                        let parsed = serde_json::from_str(raw.get())?;
                        extra.remove($key);
                        Some(parsed)
                    }
                    Some(_) | None => None,
                };)*
                Ok(Self { $($field,)* extra })
            }
        }
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    Developer,
    User,
    Assistant,
    Tool,
    Function,
}

message_record!(Message { role: Role => "role", content: Content => "content" });
message_record!(Block { kind: String => "type", text: String => "text" });
message_record!(Choice { message: Message => "message", delta: Message => "delta", text: String => "text" });
message_record!(ModelResponse { choices: Vec<Choice> => "choices" });

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged, try_from = "Box<RawValue>")]
pub enum Content {
    Text(String),
    List(Vec<Part>),
}

impl TryFrom<Box<RawValue>> for Content {
    type Error = serde_json::Error;
    fn try_from(value: Box<RawValue>) -> Result<Self, Self::Error> {
        if value.get().starts_with('[') {
            return serde_json::from_str(value.get()).map(Self::List);
        }
        serde_json::from_str(value.get()).map(Self::Text)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged, try_from = "Box<RawValue>")]
pub enum Part {
    Text(String),
    Block(Block),
    // Non-text parts are ignored by LiteLLM's extractor and removed by its masking pass.
    Other(Box<RawValue>),
}

impl TryFrom<Box<RawValue>> for Part {
    type Error = serde_json::Error;
    fn try_from(value: Box<RawValue>) -> Result<Self, Self::Error> {
        match value.get().chars().next() {
            Some('"') => serde_json::from_str(value.get()).map(Self::Text),
            Some('{') => serde_json::from_str(value.get()).map(Self::Block),
            Some(_) | None => Ok(Self::Other(value)),
        }
    }
}
