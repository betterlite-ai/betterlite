use betterlite_config::sections::cisco_verdict_contracts::InspectionType;
use serde::Serialize;
use serde_json::value::RawValue;

use crate::cisco_configuration::records::Direction;
use crate::cisco_conversation::Fields;
use crate::cisco_conversation::json::Json;

// Small serialization and labeling helpers shared by every function in this module.

pub(crate) fn rule_name(rule: &Fields) -> Option<String> {
    match Json::field(rule, "rule_name") {
        Json::Text(name) => Some(name),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

pub(crate) const fn surface_label(surface: InspectionType) -> &'static str {
    match surface {
        InspectionType::Chat => "chat",
        InspectionType::Mcp => "mcp",
    }
}

pub(crate) const fn direction_label(direction: Direction) -> &'static str {
    match direction {
        Direction::Input => "input",
        Direction::Output => "output",
    }
}

pub(crate) fn insert_serialized<Value>(fields: &mut Fields, key: &str, value: &Value)
where
    Value: Serialize + ?Sized,
{
    let Ok(text) = serde_json::to_string(value) else {
        return;
    };
    if let Ok(raw) = RawValue::from_string(text) {
        fields.insert(key.to_owned(), raw);
    }
}

mod tests;
