use serde::{Deserialize, Serialize};

macro_rules! record {
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
}

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
