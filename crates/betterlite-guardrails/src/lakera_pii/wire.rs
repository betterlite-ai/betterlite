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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(try_from = "Box<RawValue>")]
pub struct LakeraResponse {
    pub flagged: Option<bool>,
    pub payload: Option<Vec<PayloadItem>>,
    pub breakdown: Option<Vec<BreakdownItem>>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Box<RawValue>>,
}

#[derive(Default, Deserialize, Serialize)]
struct LakeraResponseHead {
    #[serde(default)]
    flagged: Option<bool>,
    #[serde(default)]
    payload: Option<Vec<PayloadItem>>,
    #[serde(default)]
    breakdown: Option<Vec<BreakdownItem>>,
}

impl TryFrom<Box<RawValue>> for LakeraResponse {
    type Error = serde_json::Error;

    fn try_from(raw: Box<RawValue>) -> Result<Self, Self::Error> {
        let (head, extra) = split_fields::<LakeraResponseHead>(raw.get())?;
        Ok(Self {
            flagged: head.flagged,
            payload: head.payload,
            breakdown: head.breakdown,
            extra,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "Box<RawValue>")]
pub struct PayloadItem {
    pub message_id: Option<u64>,
    pub start: Option<u64>,
    pub end: Option<u64>,
    pub detector_type: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Box<RawValue>>,
}

#[derive(Deserialize, Serialize)]
struct PayloadItemHead {
    #[serde(default)]
    message_id: Option<u64>,
    #[serde(default)]
    start: Option<u64>,
    #[serde(default)]
    end: Option<u64>,
    #[serde(default)]
    detector_type: Option<String>,
}

impl TryFrom<Box<RawValue>> for PayloadItem {
    type Error = serde_json::Error;

    fn try_from(raw: Box<RawValue>) -> Result<Self, Self::Error> {
        let (head, extra) = split_fields::<PayloadItemHead>(raw.get())?;
        Ok(Self {
            message_id: head.message_id,
            start: head.start,
            end: head.end,
            detector_type: head.detector_type,
            extra,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "Box<RawValue>")]
pub struct BreakdownItem {
    pub detected: Option<bool>,
    pub detector_type: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Box<RawValue>>,
}

#[derive(Deserialize, Serialize)]
struct BreakdownItemHead {
    #[serde(default)]
    detected: Option<bool>,
    #[serde(default)]
    detector_type: Option<String>,
}

impl TryFrom<Box<RawValue>> for BreakdownItem {
    type Error = serde_json::Error;

    fn try_from(raw: Box<RawValue>) -> Result<Self, Self::Error> {
        let (head, extra) = split_fields::<BreakdownItemHead>(raw.get())?;
        Ok(Self {
            detected: head.detected,
            detector_type: head.detector_type,
            extra,
        })
    }
}

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
