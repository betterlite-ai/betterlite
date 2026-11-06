use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

// A dynamic body param replaces the typed field of the same name, so the builder leaves that field out.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AnalyzeRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ad_hoc_recognizers: Option<Vec<GuardrailValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entities: Option<Vec<String>>,
    #[serde(flatten)]
    pub dynamic_params: BTreeMap<String, GuardrailValue>,
}

// analysis_explanation, recognition_metadata and any other analyzer field reach the anonymizer as the analyzer wrote them.
#[derive(Debug, Clone, Serialize)]
pub struct AnalyzeResponseItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(flatten)]
    pub other: BTreeMap<String, Box<RawValue>>,
}

fn take_field<T>(
    fields: &mut BTreeMap<String, Box<RawValue>>,
    name: &str,
) -> Result<Option<T>, serde_json::Error>
where
    T: DeserializeOwned,
{
    fields
        .remove(name)
        .map_or(Ok(None), |raw| serde_json::from_str(raw.get()))
}

impl AnalyzeResponseItem {
    // serde's flatten buffers every field, and with arbitrary_precision a buffered float no longer reads as f64, so the typed fields are lifted out of a plain map.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        let mut fields: BTreeMap<String, Box<RawValue>> = serde_json::from_str(json)?;
        Ok(Self {
            entity_type: take_field(&mut fields, "entity_type")?,
            start: take_field(&mut fields, "start")?,
            end: take_field(&mut fields, "end")?,
            score: take_field(&mut fields, "score")?,
            other: fields,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MockRedactedItem {
    #[serde(default)]
    pub entity_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MockRedactedText {
    pub text: String,
    #[serde(default)]
    pub items: Vec<MockRedactedItem>,
}

#[derive(Debug, Clone)]
pub enum AnalyzeOutcome {
    Items(Vec<AnalyzeResponseItem>),
    Redacted(MockRedactedText),
}

#[derive(Debug)]
pub enum ListEntry {
    Item(AnalyzeResponseItem),
    Invalid(Box<RawValue>),
}

#[derive(Debug)]
pub enum AnalyzeBody {
    List(Vec<ListEntry>),
    ErrorObject { error: Box<RawValue> },
    Item(AnalyzeResponseItem),
    UnparsedDict,
    Unexpected(Box<RawValue>),
}

#[derive(Deserialize)]
struct ErrorProbe {
    error: Box<RawValue>,
}

impl AnalyzeBody {
    pub fn from_raw(raw: Box<RawValue>) -> Result<Self, serde_json::Error> {
        match raw.get().as_bytes().first() {
            Some(b'[') => {
                let entries: Vec<Box<RawValue>> = serde_json::from_str(raw.get())?;
                Ok(Self::List(
                    entries
                        .into_iter()
                        .map(|entry| {
                            AnalyzeResponseItem::from_json(entry.get())
                                .map_or_else(|_| ListEntry::Invalid(entry), ListEntry::Item)
                        })
                        .collect(),
                ))
            }
            Some(b'{') => {
                if let Ok(probe) = serde_json::from_str::<ErrorProbe>(raw.get()) {
                    return Ok(Self::ErrorObject { error: probe.error });
                }
                Ok(
                    AnalyzeResponseItem::from_json(raw.get())
                        .map_or(Self::UnparsedDict, Self::Item),
                )
            }
            _ => Ok(Self::Unexpected(raw)),
        }
    }
}
