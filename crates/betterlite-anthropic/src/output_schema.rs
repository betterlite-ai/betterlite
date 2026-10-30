use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::definitions::expand::unpack_defs;

const CONSTRAINTS: [(&str, &str); 25] = [
    ("minItems", "minimum number of items: {}"),
    ("maxItems", "maximum number of items: {}"),
    ("uniqueItems", "all array items must be unique"),
    ("contains", "array must contain an item matching: {}"),
    ("minContains", "minimum number of matching items: {}"),
    ("maxContains", "maximum number of matching items: {}"),
    ("prefixItems", "leading items must match, in order: {}"),
    ("minimum", "minimum value: {}"),
    ("maximum", "maximum value: {}"),
    ("exclusiveMinimum", "exclusive minimum value: {}"),
    ("exclusiveMaximum", "exclusive maximum value: {}"),
    ("multipleOf", "must be a multiple of {}"),
    ("minLength", "minimum length: {}"),
    ("maxLength", "maximum length: {}"),
    ("minProperties", "minimum number of properties: {}"),
    ("maxProperties", "maximum number of properties: {}"),
    (
        "patternProperties",
        "properties whose names match each pattern must satisfy: {}",
    ),
    ("propertyNames", "property names must satisfy: {}"),
    ("dependentRequired", "dependent required properties: {}"),
    ("dependentSchemas", "dependent schemas: {}"),
    (
        "unevaluatedProperties",
        "unevaluated properties must satisfy: {}",
    ),
    ("if", "conditional (if): {}"),
    ("then", "conditional (then): {}"),
    ("else", "conditional (else): {}"),
    ("not", "must not match: {}"),
];

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct OutputFormat {
    #[serde(rename = "type")]
    pub type_name: String,
    pub schema: Json,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseFormat {
    Text,
    JsonSchema { schema: Json },
    JsonObject { schema: Option<Json> },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Tool {
    pub name: String,
    pub input_schema: Json,
}

pub fn filter_anthropic_output_schema(input: Json) -> Json {
    let Json::Object(schema) = input else {
        return input;
    };
    let note = constraint_note(&schema);
    let drop_type = enum_type_conflict(&schema);
    let mut result = Object::new();
    if let Some(note_text) = &note {
        let description = schema
            .get("description")
            .and_then(Json::as_str)
            .filter(|text| !text.is_empty());
        result.insert(
            "description".to_owned(),
            Json::Text(
                description.map_or_else(|| note_text.clone(), |text| format!("{text} {note_text}")),
            ),
        );
    }
    for (key, value) in schema {
        filter_field(&mut result, key, value, note.as_ref(), drop_type);
    }
    if result.get("type").and_then(Json::as_str) == Some("object") {
        result.insert("additionalProperties".to_owned(), Json::Bool(false));
    }
    Json::Object(result)
}

fn filter_field(
    result: &mut Object,
    key: String,
    value: Json,
    note: Option<&String>,
    drop_type: bool,
) {
    if CONSTRAINTS.iter().any(|(name, _)| *name == key)
        || (key == "type" && drop_type)
        || (key == "description" && note.is_some())
    {
        return;
    }
    match key.as_str() {
        "properties" | "$defs" => {
            result.insert(key, filter_properties(value));
        }
        "items" => {
            result.insert(key, filter_node(value));
        }
        "anyOf" | "allOf" | "oneOf" => add_alternative(result, &key, value),
        _ => {
            result.insert(key, value);
        }
    }
}

fn filter_properties(value: Json) -> Json {
    match value {
        Json::Object(fields) => Json::Object(
            fields
                .into_iter()
                .map(|(key, child)| (key, filter_anthropic_output_schema(child)))
                .collect(),
        ),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => value,
    }
}

fn filter_node(value: Json) -> Json {
    match value {
        Json::Object(_) => filter_anthropic_output_schema(value),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => value,
    }
}

fn add_alternative(result: &mut Object, key: &str, value: Json) {
    let Json::List(items) = value else {
        result.insert(key.to_owned(), value);
        return;
    };
    let filtered = items
        .into_iter()
        .map(filter_anthropic_output_schema)
        .collect::<Vec<_>>();
    if key == "oneOf" {
        let mut any = take_list(result, "anyOf");
        any.extend(filtered);
        result.insert("anyOf".to_owned(), Json::List(any));
    } else {
        result.insert(key.to_owned(), Json::List(filtered));
    }
}

fn take_list(result: &mut Object, key: &str) -> Vec<Json> {
    match result.shift_remove(key) {
        Some(Json::List(items)) => items,
        Some(value) => {
            result.insert(key.to_owned(), value);
            Vec::new()
        }
        None => Vec::new(),
    }
}

fn constraint_note(schema: &Object) -> Option<String> {
    let notes = CONSTRAINTS
        .iter()
        .filter_map(|(key, label)| {
            let value = schema.get(*key)?;
            (!matches!(value, Json::Bool(false))).then(|| label.replace("{}", &format_value(value)))
        })
        .collect::<Vec<_>>();
    (!notes.is_empty()).then(|| format!("Note: {}.", notes.join(", ")))
}

fn format_value(value: &Json) -> String {
    match value {
        Json::Null => "None".to_owned(),
        Json::Bool(flag) => flag.to_string().to_ascii_uppercase(),
        Json::Number(number) => number.to_string(),
        Json::Text(text) => text.clone(),
        Json::List(_) | Json::Object(_) => serde_json::to_string(value).unwrap_or_default(),
    }
}

fn enum_type_conflict(schema: &Object) -> bool {
    let Some(Json::List(values)) = schema.get("enum") else {
        return false;
    };
    let Some(Json::Text(kind)) = schema.get("type") else {
        return false;
    };
    if values.is_empty() || kind == "array" || kind == "object" {
        return true;
    }
    values.iter().any(|value| !matches_type(value, kind))
}

fn matches_type(value: &Json, kind: &str) -> bool {
    match kind {
        "null" => matches!(value, Json::Null),
        "boolean" => matches!(value, Json::Bool(_)),
        "integer" => matches!(value, Json::Number(number) if number.is_i64() || number.is_u64()),
        "number" => matches!(value, Json::Number(_)),
        "string" => matches!(value, Json::Text(_)),
        "array" => matches!(value, Json::List(_)),
        "object" => matches!(value, Json::Object(_)),
        _ => true,
    }
}

#[must_use]
pub fn map_output(value: Option<ResponseFormat>) -> Option<OutputFormat> {
    let schema = match value? {
        ResponseFormat::Text | ResponseFormat::JsonObject { schema: None } => return None,
        ResponseFormat::JsonSchema { schema }
        | ResponseFormat::JsonObject {
            schema: Some(schema),
        } => schema,
    };
    Some(OutputFormat {
        type_name: "json_schema".to_owned(),
        schema: filter_anthropic_output_schema(expand_definitions(schema)),
    })
}

fn expand_definitions(schema: Json) -> Json {
    let Json::Object(mut fields) = schema else {
        return schema;
    };
    let defs = fields.shift_remove("$defs");
    let legacy = fields.shift_remove("definitions");
    let Some(Json::Object(definition_fields)) = defs.or(legacy) else {
        return Json::Object(fields);
    };
    if definition_fields.is_empty() {
        return Json::Object(fields);
    }
    unpack_defs(fields, &definition_fields, Some(10_000_000))
        .ok()
        .map_or_else(|| Json::Object(Object::new()), Json::Object)
}

#[must_use]
pub fn map_tool(value: Option<ResponseFormat>) -> Option<Tool> {
    let schema = match value? {
        ResponseFormat::Text | ResponseFormat::JsonObject { schema: None } => return None,
        ResponseFormat::JsonSchema { schema }
        | ResponseFormat::JsonObject {
            schema: Some(schema),
        } => schema,
    };
    Some(json_tool(Some(schema)))
}

#[must_use]
pub fn json_tool(schema: Option<Json>) -> Tool {
    let input_schema = schema.map_or_else(
        || {
            Json::Object(Object::from_iter([
                (String::from("type"), Json::Text(String::from("object"))),
                (String::from("additionalProperties"), Json::Bool(true)),
                (String::from("properties"), Json::Object(Object::new())),
            ]))
        },
        |provided| {
            let mut base =
                Object::from_iter([(String::from("type"), Json::Text(String::from("object")))]);
            if let Json::Object(fields) = provided {
                base.extend(fields);
            }
            Json::Object(base)
        },
    );
    Tool {
        name: "json_tool_call".to_owned(),
        input_schema,
    }
}

mod tests;
