use serde::{Deserialize, Serialize};

macro_rules! record {
    (union $name:ident [tag = $tag:literal] { $($literal:literal => $variant:ident($kind:ty)),* $(,)? }) => {
        #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
        #[serde(untagged)]
        pub enum $name {
            $($variant($kind),)*
            Other(serde_json::Value),
        }
        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where D: serde::Deserializer<'de> {
                let value = serde_json::Value::deserialize(deserializer)?;
                match value.get($tag).and_then(serde_json::Value::as_str) {
                    $(Some($literal) => serde_json::from_value(value.clone()).map_or_else(
                        |_| Ok(Self::Other(value.clone())),
                        |record| Ok(Self::$variant(record)),
                    ),)*
                    _ => Ok(Self::Other(value)),
                }
            }
        }
    };
    ($name:ident: $tag:ident = $wire:literal { $($fields:tt)* }) => {
        strings! { $tag { Value = $wire } }
        record! { $name { r#type: $tag, $($fields)* } }
    };
    ($name:ident { $($(#[$attribute:meta])* $field:ident: $kind:ty),* $(,)? }) => {
        #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
        pub struct $name {
            $($(#[$attribute])* pub $field: $kind,)*
            #[serde(flatten)]
            pub extra: std::collections::BTreeMap<String, betterlite_models::decisions::Json>,
        }
    };
    ($name:ident { $($field:ident: $kind:ty => $wire:literal),* $(,)? }) => {
        #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, Default)]
        pub struct $name {
            $(#[serde(rename = $wire, skip_serializing_if = "Option::is_none")] pub $field: Option<$kind>,)*
            #[serde(flatten)]
            pub extra: std::collections::BTreeMap<String, serde_json::Value>,
        }
        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where D: serde::Deserializer<'de> {
                let mut fields = std::collections::BTreeMap::<String, serde_json::Value>::deserialize(deserializer)?;
                let mut record = Self::default();
                $(if let Some(value) = fields.remove($wire) {
                    if value.is_null() {
                        record.extra.insert($wire.to_owned(), value);
                    } else {
                        match serde_json::from_value(value.clone()) {
                            Ok(typed) => record.$field = Some(typed),
                            Err(_) => { record.extra.insert($wire.to_owned(), value); }
                        }
                    }
                })*
                record.extra.extend(fields);
                Ok(record)
            }
        }
    };
}

pub(crate) use record;

macro_rules! strings {
    ($name:ident { $($variant:ident = $wire:literal),* $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
        pub enum $name {
            $(#[serde(rename = $wire)] $variant,)*
        }
    };
}

macro_rules! union {
    ($name:ident { $($variant:ident($kind:ty)),* $(,)? }) => {
        #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
        #[serde(untagged)]
        pub enum $name {
            $($variant($kind),)*
        }
    };
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Number(pub serde_json::Number);

pub mod computer;
pub mod literals;
pub mod mcp;
pub mod output;
pub mod response;
pub mod search;
pub mod shell;
pub mod text;
pub mod tools;
pub mod unions;
