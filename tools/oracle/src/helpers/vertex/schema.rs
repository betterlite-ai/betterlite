use std::collections::BTreeMap;

use betterlite_config::sections::Section as _;
use betterlite_config::sections::vertex_schema_shaping::Raw;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_vertex::vertex_schema::build::{SchemaFormat, shape_parameters};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::helpers::{Helper, HelperResult, OrHelperError as _};

mod tests;

pub(crate) struct Op;

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct Input {
    mode: Mode,
    schema: Object,
    add_property_ordering: bool,
    settings: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Mode {
    BuildVertexSchema,
    BuildJsonSchema,
}

impl Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        let settings = Raw::default().convert(&input.settings).or_config_error()?;
        let format = match input.mode {
            Mode::BuildVertexSchema => SchemaFormat::ResponseSchema,
            Mode::BuildJsonSchema => SchemaFormat::JsonSchema,
        };
        let shaped = shape_parameters(
            input.schema.clone(),
            format,
            input.add_property_ordering,
            &settings,
        )
        .or_value_error()?;
        Ok(object_to_value(shaped))
    }
}

fn json_to_value(json: Json) -> Value {
    match json {
        Json::Null => Value::Null,
        Json::Bool(flag) => Value::Bool(flag),
        Json::Number(number) => Value::Number(number),
        Json::Text(text) => Value::String(text),
        Json::List(items) => Value::Array(items.into_iter().map(json_to_value).collect()),
        Json::Object(fields) => object_to_value(fields),
    }
}

fn object_to_value(object: Object) -> Value {
    let entries: Map<String, Value> = object
        .into_iter()
        .map(|(key, value)| (key, json_to_value(value)))
        .collect();
    Value::Object(entries)
}
