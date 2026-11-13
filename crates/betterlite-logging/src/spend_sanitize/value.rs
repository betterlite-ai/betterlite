use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(u64);

impl From<u64> for NodeId {
    fn from(raw: u64) -> Self {
        Self(raw)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpendList {
    pub node: NodeId,
    pub items: Vec<SpendValue>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpendMapping {
    pub node: NodeId,
    pub entries: IndexMap<String, SpendValue>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SpendValue {
    Null,
    Bool(bool),
    Number(Number),
    Text(String),
    List(SpendList),
    Mapping(SpendMapping),
    Model(SpendMapping),
    Object(SpendMapping),
}

impl SpendValue {
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            Self::Null
            | Self::Bool(_)
            | Self::Number(_)
            | Self::List(_)
            | Self::Mapping(_)
            | Self::Model(_)
            | Self::Object(_) => None,
        }
    }
}

impl From<&Json> for SpendValue {
    fn from(value: &Json) -> Self {
        spend_value(value, &mut 0_u64)
    }
}

fn spend_value(value: &Json, next: &mut u64) -> SpendValue {
    match value {
        Json::Null => SpendValue::Null,
        Json::Bool(flag) => SpendValue::Bool(*flag),
        Json::Number(number) => SpendValue::Number(number.clone()),
        Json::Text(text) => SpendValue::Text(text.clone()),
        Json::List(items) => {
            let node = NodeId(*next);
            *next = next.saturating_add(1);
            let converted = items
                .iter()
                .map(|item| spend_value(item, next))
                .collect::<Vec<_>>();
            SpendValue::List(SpendList {
                node,
                items: converted,
            })
        }
        Json::Object(entries) => {
            let node = NodeId(*next);
            *next = next.saturating_add(1);
            let converted = entries
                .iter()
                .map(|(key, item)| (key.clone(), spend_value(item, next)))
                .collect::<IndexMap<_, _>>();
            SpendValue::Mapping(SpendMapping {
                node,
                entries: converted,
            })
        }
    }
}

// A foreign Python model or object has no JSON form of its own; its fields are the closest projection
pub fn to_json(value: &SpendValue) -> Json {
    match value {
        SpendValue::Null => Json::Null,
        SpendValue::Bool(flag) => Json::Bool(*flag),
        SpendValue::Number(number) => Json::Number(number.clone()),
        SpendValue::Text(text) => Json::Text(text.clone()),
        SpendValue::List(list) => Json::List(list.items.iter().map(to_json).collect()),
        SpendValue::Mapping(mapping) | SpendValue::Model(mapping) | SpendValue::Object(mapping) => {
            Json::Object(
                mapping
                    .entries
                    .iter()
                    .map(|(key, item)| (key.clone(), to_json(item)))
                    .collect(),
            )
        }
    }
}
