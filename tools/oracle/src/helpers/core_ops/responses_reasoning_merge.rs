use betterlite_core::api::responses_reasoning_merge::merge_reasoning;
use betterlite_core::api::responses_reasoning_merge::record::{ChatRecord, Record};
use betterlite_core::api::responses_reasoning_merge::trailing::merge_trailing_values;
use serde::Deserialize;
use serde_json::Value;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Mode {
    Reasoning,
    Trailing,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RecordKind {
    Dict,
    Attribute,
}

// LiteLLM dispatches isinstance(msg, dict) per message, so one call can mix dict and attribute records.
#[derive(Deserialize)]
#[serde(untagged)]
pub(crate) enum RecordKinds {
    Single(RecordKind),
    PerMessage(Vec<RecordKind>),
}

impl Default for RecordKinds {
    fn default() -> Self {
        Self::Single(RecordKind::Dict)
    }
}

impl RecordKinds {
    fn resolve(&self, count: usize) -> Vec<RecordKind> {
        match self {
            Self::Single(kind) => vec![*kind; count],
            Self::PerMessage(kinds) => kinds.clone(),
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct Input {
    mode: Mode,
    #[serde(default)]
    record_kind: RecordKinds,
    messages: Vec<Value>,
    #[serde(default)]
    new_messages: Vec<Value>,
    #[serde(default)]
    hosted_search: bool,
}

fn to_records(
    messages: &[Value],
    kinds: &[RecordKind],
) -> Result<Vec<Record>, Box<crate::helpers::HelperError>> {
    if kinds.len() != messages.len() {
        return Err(Box::new(crate::helpers::HelperError {
            class: "InvalidInput".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: "record_kind length must match messages length".to_owned(),
        }));
    }
    messages
        .iter()
        .cloned()
        .zip(kinds.iter())
        .map(|(value, kind)| {
            let record: ChatRecord = serde_json::from_value(value)?;
            Ok(match kind {
                RecordKind::Dict => Record::Dict(record),
                RecordKind::Attribute => Record::Attribute(record),
            })
        })
        .collect::<Result<_, serde_json::Error>>()
        .map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
}

fn merge_reasoning_records(
    messages: &[Value],
    kinds: &[RecordKind],
) -> crate::helpers::HelperResult {
    let mut records = to_records(messages, kinds)?;
    let merged = merge_reasoning(&mut records);
    let values: Vec<Value> = merged
        .iter()
        .map(|record| {
            let (Record::Dict(inner) | Record::Attribute(inner)) = record;
            serde_json::to_value(inner)
        })
        .collect::<Result<_, serde_json::Error>>()
        .map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
    Ok(Value::Array(values))
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input.mode {
            Mode::Reasoning => {
                let kinds = input.record_kind.resolve(input.messages.len());
                merge_reasoning_records(&input.messages, &kinds)
            }
            Mode::Trailing => {
                merge_trailing_values(&input.messages, &input.new_messages, input.hosted_search)
                    .map(|merged| {
                        // keep the "merged" key present so a null result round-trips, not a missing key
                        let mut object = serde_json::Map::new();
                        object.insert("merged".to_owned(), merged.unwrap_or(Value::Null));
                        Value::Object(object)
                    })
                    .map_err(|error| {
                        Box::new(crate::helpers::HelperError {
                            class: "InvalidInput".to_owned(),
                            status: None,
                            error_type: None,
                            param: None,
                            code: None,
                            message: error.to_string(),
                        })
                    })
            }
        }
    }
}
