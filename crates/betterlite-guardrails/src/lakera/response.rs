use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

pub type Extensions = BTreeMap<String, Box<RawValue>>;

// LiteLLM reads only results[0] and echoes the whole response back in the error detail. Every record
// keeps the fields it models typed and every other field, an explicit null included, in `extra`, so it
// serializes back to what it read.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "Extensions")]
pub struct LakeraModerationResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub results: Option<Vec<LakeraResult>>,
    #[serde(flatten)]
    pub extra: Extensions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "Extensions")]
pub struct LakeraResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flagged: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_scores: Option<CategoryScores>,
    #[serde(flatten)]
    pub extra: Extensions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "Extensions")]
pub struct CategoryScores {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_injection: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jailbreak: Option<f64>,
    #[serde(flatten)]
    pub extra: Extensions,
}

impl TryFrom<Extensions> for LakeraModerationResponse {
    type Error = serde_json::Error;

    fn try_from(mut extra: Extensions) -> Result<Self, Self::Error> {
        let results = take_typed(&mut extra, "results")?;
        Ok(Self { results, extra })
    }
}

impl TryFrom<Extensions> for LakeraResult {
    type Error = serde_json::Error;

    fn try_from(mut extra: Extensions) -> Result<Self, Self::Error> {
        let flagged = take_typed(&mut extra, "flagged")?;
        let category_scores = take_typed(&mut extra, "category_scores")?;
        Ok(Self {
            flagged,
            category_scores,
            extra,
        })
    }
}

impl TryFrom<Extensions> for CategoryScores {
    type Error = serde_json::Error;

    fn try_from(mut extra: Extensions) -> Result<Self, Self::Error> {
        let prompt_injection = take_typed(&mut extra, "prompt_injection")?;
        let jailbreak = take_typed(&mut extra, "jailbreak")?;
        Ok(Self {
            prompt_injection,
            jailbreak,
            extra,
        })
    }
}

fn take_typed<T>(extra: &mut Extensions, key: &str) -> Result<Option<T>, serde_json::Error>
where
    T: DeserializeOwned,
{
    let Some(raw) = extra.get(key) else {
        return Ok(None);
    };
    if raw.get().trim() == "null" {
        return Ok(None);
    }
    let parsed = serde_json::from_str(raw.get())?;
    extra.remove(key);
    Ok(Some(parsed))
}
