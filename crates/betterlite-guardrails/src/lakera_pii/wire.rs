use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

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
    ($name:ident / $head:ident { $($field:ident : $ty:ty),* $(,)? } skip) => {
        #[derive(Debug, Clone, Default, Serialize, Deserialize)]
        #[serde(try_from = "Box<RawValue>")]
        pub struct $name {
            $(#[serde(skip_serializing_if = "Option::is_none")] pub $field: $ty,)*
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

    ($name:ident / $head:ident { $($field:ident : $ty:ty),* $(,)? }) => {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        #[serde(try_from = "Box<RawValue>")]
        pub struct $name {
            $(pub $field: $ty,)*
            #[serde(flatten)]
            pub extra: BTreeMap<String, Box<RawValue>>,
        }

        #[derive(Deserialize, Serialize)]
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

raw_value_record!(LakeraResponse / LakeraResponseHead {
    flagged: Option<bool>,
    payload: Option<Vec<PayloadItem>>,
    breakdown: Option<Vec<BreakdownItem>>,
} skip);

raw_value_record!(PayloadItem / PayloadItemHead {
    message_id: Option<u64>,
    start: Option<u64>,
    end: Option<u64>,
    detector_type: Option<String>,
});

raw_value_record!(BreakdownItem / BreakdownItemHead {
    detected: Option<bool>,
    detector_type: Option<String>,
});

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "Box<RawValue>")]
pub struct Message {
    pub content: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Box<RawValue>>,
}

#[derive(Deserialize, Serialize)]
struct MessageHead {
    #[serde(default)]
    content: Option<String>,
}

impl TryFrom<Box<RawValue>> for Message {
    type Error = serde_json::Error;

    fn try_from(raw: Box<RawValue>) -> Result<Self, Self::Error> {
        let (head, extra) = split_fields::<MessageHead>(raw.get())?;
        Ok(Self {
            content: head.content,
            extra,
        })
    }
}
