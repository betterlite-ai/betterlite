use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::spend_sanitize::value::{NodeId, SpendList, SpendMapping, SpendValue};

pub(super) fn parse(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

pub(super) fn value(source: &str) -> SpendValue {
    SpendValue::from(&parse(source))
}

pub(super) fn mapping(node: u64, entries: Vec<(&str, SpendValue)>) -> SpendValue {
    SpendValue::Mapping(SpendMapping {
        node: NodeId::from(node),
        entries: entries
            .into_iter()
            .map(|(key, item)| (key.to_owned(), item))
            .collect(),
    })
}

pub(super) fn sequence(node: u64, items: Vec<SpendValue>) -> SpendValue {
    SpendValue::List(SpendList {
        node: NodeId::from(node),
        items,
    })
}

pub(super) fn text(content: &str) -> SpendValue {
    SpendValue::Text(content.to_owned())
}

pub(super) fn number(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

pub(super) fn spend_number(value: i64) -> SpendValue {
    SpendValue::Number(serde_json::Number::from(value))
}

pub(super) fn object(json: &Json) -> &IndexMap<String, Json> {
    let Json::Object(entries) = json else {
        panic!("expected an object")
    };
    entries
}

pub(super) fn lookup<'input>(json: &'input Json, path: &str) -> &'input Json {
    let mut current = json;
    for step in path.split('.') {
        current = match current {
            Json::Object(entries) => entries.get(step).expect("object field"),
            Json::List(items) => items
                .get(step.parse::<usize>().expect("list index"))
                .expect("list item"),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => {
                panic!("path goes through a scalar")
            }
        };
    }
    current
}
