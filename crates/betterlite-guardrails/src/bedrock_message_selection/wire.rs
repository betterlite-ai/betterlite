use crate::bedrock_records::Extensions;
use serde::de::{Error, SeqAccess, Visitor, value::SeqAccessDeserializer};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::value::RawValue;
use std::fmt;

// Missing fields and explicit nulls must survive positional write-back unchanged.
macro_rules! message_record {
    ($name:ident { $($field:ident: $kind:ty => $key:literal),* $(,)? }) => {
        #[derive(Debug, Clone, Serialize)]
        pub struct $name {
            $(#[serde(rename = $key, skip_serializing_if = "Option::is_none")]
            pub $field: Option<$kind>,)*
            #[serde(flatten)]
            pub extra: Extensions,
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error> where D: Deserializer<'de> {
                let mut extra = Extensions::deserialize(deserializer)?;
                $(let $field = match extra.get($key) {
                    Some(raw) if raw.get() != "null" => {
                        let parsed = serde_json::from_str(raw.get()).map_err(D::Error::custom)?;
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

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    List(Vec<Part>),
}

impl<'de> Deserialize<'de> for Content {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(ContentVisitor)
    }
}
struct ContentVisitor;
impl<'de> Visitor<'de> for ContentVisitor {
    type Value = Content;
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("message text or content blocks")
    }
    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: Error,
    {
        Ok(Content::Text(v.to_owned()))
    }
    fn visit_seq<A>(self, seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        Vec::<Part>::deserialize(SeqAccessDeserializer::new(seq)).map(Content::List)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum Part {
    Text(String),
    Block(Block),
    // Non-text parts are ignored by LiteLLM's extractor and removed by its masking pass.
    Other(Box<RawValue>),
}

impl<'de> Deserialize<'de> for Part {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = Box::<RawValue>::deserialize(deserializer)?;
        match raw.get().chars().next() {
            Some('"') => serde_json::from_str(raw.get())
                .map(Self::Text)
                .map_err(D::Error::custom),
            Some('{') => serde_json::from_str(raw.get())
                .map(Self::Block)
                .map_err(D::Error::custom),
            Some(_) | None => Ok(Self::Other(raw)),
        }
    }
}
