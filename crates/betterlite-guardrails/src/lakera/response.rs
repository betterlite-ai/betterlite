use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

pub type Extensions = BTreeMap<String, Box<RawValue>>;

// a key that is absent is None and one that is present and null is Some(None), so the 400 detail echoes the response as read
type Presence<Field> = Option<Option<Field>>;

fn take_field<Field>(fields: &mut Extensions, key: &str) -> serde_json::Result<Presence<Field>>
where
    Field: DeserializeOwned,
{
    fields
        .remove(key)
        .map(|raw| serde_json::from_str::<Option<Field>>(raw.get()))
        .transpose()
}

// Box<RawValue> cannot go through serde's flatten buffer, so each record splits its known keys off the raw object
macro_rules! split_record {
    ($record:ident { $($field:ident),+ }) => {
        impl TryFrom<Box<RawValue>> for $record {
            type Error = serde_json::Error;

            fn try_from(raw: Box<RawValue>) -> Result<Self, Self::Error> {
                let mut fields = serde_json::from_str::<Extensions>(raw.get())?;
                Ok(Self {
                    $($field: take_field(&mut fields, stringify!($field))?,)+
                    extra: fields,
                })
            }
        }
    };
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "Box<RawValue>")]
pub struct LakeraModerationResponse {
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub results: Option<Option<Vec<LakeraResult>>>,
    #[serde(flatten)]
    pub extra: Extensions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "Box<RawValue>")]
pub struct LakeraResult {
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub flagged: Option<Option<bool>>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub category_scores: Option<Option<CategoryScores>>,
    #[serde(flatten)]
    pub extra: Extensions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "Box<RawValue>")]
pub struct CategoryScores {
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub prompt_injection: Option<Option<f64>>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub jailbreak: Option<Option<f64>>,
    #[serde(flatten)]
    pub extra: Extensions,
}

split_record!(LakeraModerationResponse { results });
split_record!(LakeraResult {
    flagged,
    category_scores
});
split_record!(CategoryScores {
    prompt_injection,
    jailbreak
});
