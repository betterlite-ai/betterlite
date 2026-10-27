use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

pub type AdditionalFields = BTreeMap<String, Box<RawValue>>;

// Concrete derives avoid Serde's conflicting generic Deserialize bounds on raw conversions.
macro_rules! record {
    ($name:ident, $head:ident) => {
        #[derive(Debug, Clone, Default, Deserialize, Serialize)]
        #[serde(try_from = "Box<RawValue>")]
        pub struct $name {
            #[serde(flatten)]
            pub head: $head,
            #[serde(flatten)]
            pub rest: AdditionalFields,
        }

        impl TryFrom<Box<RawValue>> for $name {
            type Error = WireError;

            fn try_from(raw: Box<RawValue>) -> Result<Self, Self::Error> {
                let (head, rest) = split_record(raw.get())?;
                Ok(Self { head, rest })
            }
        }
    };
}

// Wrong-shaped values are passthrough data in LiteLLM, including arrays and objects.
macro_rules! tolerant {
    ($name:ident, $value:ty) => {
        #[derive(Debug, Clone, Deserialize, Serialize)]
        #[serde(untagged, from = "Box<RawValue>")]
        pub enum $name {
            Known($value),
            Other(Box<RawValue>),
        }

        impl From<Box<RawValue>> for $name {
            fn from(raw: Box<RawValue>) -> Self {
                match serde_json::from_str(raw.get()) {
                    Ok(value) => Self::Known(value),
                    Err(_) => Self::Other(raw),
                }
            }
        }
    };
}

record!(RequestParams, RequestHead);
record!(NamedRecord, NamedHead);
record!(MessageRecord, MessageHead);
record!(CallRecord, CallHead);
record!(FunctionRecord, FunctionHead);
tolerant!(Name, String);
tolerant!(Tool, NamedRecord);
tolerant!(Tools, Vec<Tool>);
tolerant!(Message, MessageRecord);
tolerant!(Call, CallRecord);
tolerant!(Calls, Vec<Call>);
tolerant!(Function, FunctionRecord);

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct RequestHead {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Tools>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<Tool>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct NamedHead {
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<Name>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<Name>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct MessageHead {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Calls>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function_call: Option<Function>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct CallHead {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function: Option<Function>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct FunctionHead {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<Name>,
}

#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("tool-name record is not a JSON object")]
    Json(#[from] serde_json::Error),
}

fn split_record<Head>(raw: &str) -> Result<(Head, AdditionalFields), WireError>
where
    Head: DeserializeOwned + Serialize,
{
    // RawValue cannot deserialize from flatten's buffer, so retain extra fields separately.
    let head = serde_json::from_str::<Head>(raw)?;
    let modeled = serde_json::from_str::<AdditionalFields>(&serde_json::to_string(&head)?)?;
    let mut rest = serde_json::from_str::<AdditionalFields>(raw)?;
    rest.retain(|key, _| !modeled.contains_key(key));
    Ok((head, rest))
}
