use betterlite_core::api::responses_reasoning_merge::{
    AttributeRecord, ChatRecord, Reasoning, merge_reasoning, merge_trailing,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Mode {
    Reasoning,
    Trailing,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RecordKind {
    #[default]
    Dict,
    Attribute,
}

#[derive(Deserialize)]
pub(crate) struct Input {
    mode: Mode,
    #[serde(default)]
    record_kind: RecordKind,
    messages: Vec<Value>,
    #[serde(default)]
    new_messages: Vec<Value>,
    #[serde(default)]
    hosted_search: bool,
}

fn merge_reasoning_records<T>(messages: &[Value]) -> crate::helpers::HelperResult
where
    T: Reasoning + Clone + Serialize + DeserializeOwned,
{
    let mut records: Vec<T> = messages
        .iter()
        .cloned()
        .map(serde_json::from_value)
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
    serde_json::to_value(merge_reasoning(&mut records)).map_err(|error| {
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

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input.mode {
            Mode::Reasoning => match input.record_kind {
                RecordKind::Dict => merge_reasoning_records::<ChatRecord>(&input.messages),
                RecordKind::Attribute => {
                    merge_reasoning_records::<AttributeRecord>(&input.messages)
                }
            },
            Mode::Trailing => {
                merge_trailing(&input.messages, &input.new_messages, input.hosted_search)
                    .map(|merged| {
                        // a bare null here would collapse to a missing "value" key once the wire format round-trips through Option<Value>
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
