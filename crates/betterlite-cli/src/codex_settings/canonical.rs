use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;
use toml_edit::{Item, Value};

pub(crate) fn to_json(item: &Item) -> Json {
    // into_value recurses through nested tables, turning every item into a plain value or into none
    item.clone()
        .into_value()
        .map_or(Json::Null, |value| value_to_json(&value))
}

fn value_to_json(value: &Value) -> Json {
    match value {
        Value::String(text) => Json::Text(text.value().clone()),
        Value::Integer(number) => Json::Number(Number::from(*number.value())),
        Value::Float(number) => float_to_json(*number.value()),
        Value::Boolean(flag) => Json::Bool(*flag.value()),
        Value::Datetime(moment) => Json::Text(moment.value().to_string()),
        Value::Array(array) => Json::List(array.iter().map(value_to_json).collect()),
        Value::InlineTable(table) => {
            sorted_object(table.iter().map(|(key, child)| (key, value_to_json(child))))
        }
    }
}

// standard JSON has no NaN or Infinity, so a non-finite TOML float keeps a marked string of its own
fn float_to_json(value: f64) -> Json {
    Number::from_f64(value).map_or_else(
        || Json::Text(format!("\u{0}toml-float:{value}")),
        Json::Number,
    )
}

// json.dumps sorts object keys by code point
fn sorted_object<'pairs, Pairs>(pairs: Pairs) -> Json
where
    Pairs: Iterator<Item = (&'pairs str, Json)>,
{
    let sorted: BTreeMap<&str, Json> = pairs.collect();
    Json::Object(
        sorted
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect::<IndexMap<String, Json>>(),
    )
}
