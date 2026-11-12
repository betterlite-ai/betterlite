#![cfg(test)]

use serde_json::{Map, Value};
use uuid::Uuid;

use crate::chat_choices::wire::TransformedChoice;
use crate::chat_choices::{ChatChoicesError, decode_chat_choices};
use crate::openai_base_config::Object;

pub(super) fn nil_id() -> Uuid {
    Uuid::nil()
}

pub(super) fn fixed_clock() -> i64 {
    7
}

pub(super) fn value(fields: &[(&str, Value)]) -> Value {
    Value::Object(
        fields
            .iter()
            .map(|(name, field)| ((*name).to_owned(), field.clone()))
            .collect::<Map<String, Value>>(),
    )
}

pub(super) fn reply(choices: &[Value]) -> Vec<u8> {
    serde_json::to_vec(&value(&[("choices", Value::Array(choices.to_vec()))])).unwrap()
}

pub(super) fn parse(
    choices: &[Value],
    optional_params: Option<&Object>,
    json_mode: bool,
) -> Result<Vec<TransformedChoice>, ChatChoicesError> {
    decode_chat_choices(&reply(choices), optional_params, json_mode, nil_id)
}

pub(super) fn optional_params(fields: &[(&str, Value)]) -> Object {
    fields
        .iter()
        .map(|(name, field)| ((*name).to_owned(), field.clone()))
        .collect::<Map<String, Value>>()
}

pub(super) fn tools(names: &[&str]) -> Object {
    let entries = names
        .iter()
        .map(|name| {
            value(&[
                ("type", Value::String(String::from("function"))),
                (
                    "function",
                    value(&[
                        ("name", Value::String((*name).to_owned())),
                        (
                            "parameters",
                            value(&[("type", Value::String(String::from("object")))]),
                        ),
                    ]),
                ),
            ])
        })
        .collect::<Vec<_>>();
    optional_params(&[("tools", Value::Array(entries))])
}

pub(super) fn text(text: &str) -> Value {
    Value::String(text.to_owned())
}

pub(super) fn choice(index: i64, finish_reason: Option<&str>, message: Value) -> Value {
    let finish = finish_reason.map_or(Value::Null, text);
    value(&[
        ("index", Value::from(index)),
        ("finish_reason", finish),
        ("message", message),
    ])
}

pub(super) fn tool_call(id: Option<&str>, name: &str, arguments: &str) -> Value {
    let call_id = id.map_or(Value::Null, text);
    value(&[
        ("id", call_id),
        ("type", Value::String(String::from("function"))),
        (
            "function",
            value(&[("name", text(name)), ("arguments", text(arguments))]),
        ),
    ])
}
