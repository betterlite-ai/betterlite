use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

pub type Extensions = BTreeMap<String, Box<RawValue>>;

// A key that is absent is None, and a key that is present and null is Some(None), so the 400 detail echoes the response as read.
// Box<RawValue> cannot go through serde's flatten buffer, so each record splits its known keys off the raw object and keeps the rest verbatim.
fn take_field<Field>(
    fields: &mut Extensions,
    key: &str,
) -> serde_json::Result<Option<Option<Field>>>
where
    Field: DeserializeOwned,
{
    fields
        .remove(key)
        .map(|raw| serde_json::from_str::<Option<Field>>(raw.get()))
        .transpose()
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

impl TryFrom<Box<RawValue>> for LakeraModerationResponse {
    type Error = serde_json::Error;

    fn try_from(raw: Box<RawValue>) -> Result<Self, Self::Error> {
        let mut fields = serde_json::from_str::<Extensions>(raw.get())?;
        Ok(Self {
            results: take_field(&mut fields, "results")?,
            extra: fields,
        })
    }
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

impl TryFrom<Box<RawValue>> for LakeraResult {
    type Error = serde_json::Error;

    fn try_from(raw: Box<RawValue>) -> Result<Self, Self::Error> {
        let mut fields = serde_json::from_str::<Extensions>(raw.get())?;
        Ok(Self {
            flagged: take_field(&mut fields, "flagged")?,
            category_scores: take_field(&mut fields, "category_scores")?,
            extra: fields,
        })
    }
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

impl TryFrom<Box<RawValue>> for CategoryScores {
    type Error = serde_json::Error;

    fn try_from(raw: Box<RawValue>) -> Result<Self, Self::Error> {
        let mut fields = serde_json::from_str::<Extensions>(raw.get())?;
        Ok(Self {
            prompt_injection: take_field(&mut fields, "prompt_injection")?,
            jailbreak: take_field(&mut fields, "jailbreak")?,
            extra: fields,
        })
    }
}
