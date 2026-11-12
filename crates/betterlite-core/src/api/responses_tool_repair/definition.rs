use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use super::error::RepairError;
use super::value::Value;
use crate::api::responses_codex_tools::custom_tools::serialize_tool_call_arguments;

// With no cached definition LiteLLM takes the first tool that has a name, so the name need not match the result id.
pub fn reconstruct_from_tools(
    tool_call_id: &str,
    tools: &[Value],
) -> Result<Option<Value>, RepairError> {
    for tool in tools {
        let Value::Dict(tool_map) = tool else {
            continue;
        };
        let function_name = match tool_map
            .get("function")
            .filter(|function| function.has_content())
        {
            None => None,
            Some(Value::Dict(function)) => function.get("name"),
            Some(_) => return Err(RepairError::WrongType { field: "function" }),
        };
        let name = function_name
            .filter(|name| name.has_content())
            .or_else(|| tool_map.get("name").filter(|name| name.has_content()));
        if let Some(found) = name {
            return Ok(Some(Value::Dict(IndexMap::from([
                ("id".to_owned(), Value::Text(tool_call_id.to_owned())),
                ("type".to_owned(), Value::Text("function".to_owned())),
                (
                    "function".to_owned(),
                    Value::Dict(IndexMap::from([
                        ("name".to_owned(), found.clone()),
                        ("arguments".to_owned(), Value::Text("{}".to_owned())),
                    ])),
                ),
            ]))));
        }
    }
    Ok(None)
}

// A function object that exposes a name or arguments becomes a plain dict; anything else is left as it is.
fn convert_function(function: &Value) -> Result<Option<Value>, RepairError> {
    if matches!(function, Value::Null | Value::Dict(_)) {
        return Ok(None);
    }
    let name = function.field("name")?;
    let arguments = function.field("arguments")?;
    if name.is_none() && arguments.is_none() {
        return Ok(None);
    }
    Ok(Some(Value::Dict(IndexMap::from([
        ("name".to_owned(), name.cloned().unwrap_or(Value::Null)),
        (
            "arguments".to_owned(),
            arguments.cloned().unwrap_or(Value::Null),
        ),
    ]))))
}

pub fn normalize_definition(
    definition: &Value,
    tool_call_id: &str,
) -> Result<Option<Value>, RepairError> {
    if !definition.has_content() {
        return Ok(None);
    }
    let mut normalized = if let Value::Dict(map) = definition {
        map.clone()
    } else {
        let id = definition.field("id")?;
        let kind = definition.field("type")?;
        let function = definition.field("function")?;
        if id.is_none() && kind.is_none() && function.is_none() {
            return Ok(None);
        }
        IndexMap::from([
            ("id".to_owned(), id.cloned().unwrap_or(Value::Null)),
            ("type".to_owned(), kind.cloned().unwrap_or(Value::Null)),
            (
                "function".to_owned(),
                function.cloned().unwrap_or(Value::Null),
            ),
        ])
    };
    let converted = match normalized.get("function") {
        Some(function) => convert_function(function)?,
        None => None,
    };
    if let Some(function) = converted {
        normalized.insert("function".to_owned(), function);
    }
    for (key, fallback) in [("id", tool_call_id), ("type", "function")] {
        if !normalized.get(key).is_some_and(Value::has_content) {
            normalized.insert(key.to_owned(), Value::Text(fallback.to_owned()));
        }
    }
    Ok(Some(Value::Dict(normalized)))
}

pub fn create_tool_call_chunk(
    definition: &Value,
    tool_call_id: &str,
    index: usize,
) -> Result<Value, RepairError> {
    let function = definition.field("function")?.unwrap_or(&Value::Null);
    let name = match function.field("name")?.filter(|name| name.has_content()) {
        Some(found) => found.text("name")?,
        None => String::new(),
    };
    let arguments = match function.field("arguments")?.filter(|raw| raw.has_content()) {
        Some(raw) => serialize_tool_call_arguments(&to_json(raw, "arguments")?, "{}"),
        None => "{}".to_owned(),
    };
    let id = match definition.field("id")? {
        Some(raw) => raw.text("id")?,
        None => tool_call_id.to_owned(),
    };
    let kind = match definition.field("type")? {
        Some(raw) => raw.text("type")?,
        None => "function".to_owned(),
    };
    Ok(Value::Dict(IndexMap::from([
        ("id".to_owned(), Value::Text(id)),
        ("type".to_owned(), Value::Text(kind)),
        (
            "function".to_owned(),
            Value::Dict(IndexMap::from([
                ("name".to_owned(), Value::Text(name)),
                ("arguments".to_owned(), Value::Text(arguments)),
            ])),
        ),
        ("index".to_owned(), Value::Number(Number::from(index))),
    ])))
}

// A dict gets a missing tool_calls list but keeps a null one; an object turns a null into a list.
pub fn append_tool_call(assistant: &mut Value, chunk: Value) {
    match assistant {
        Value::Dict(map) => {
            let slot = map
                .entry("tool_calls".to_owned())
                .or_insert_with(|| Value::List(Vec::new()));
            if let Value::List(calls) = slot {
                calls.push(chunk);
            }
        }
        Value::Instance { object } => {
            if let Some(slot) = object.attrs.get_mut("tool_calls") {
                if *slot == Value::Null {
                    *slot = Value::List(Vec::new());
                }
                if let Value::List(calls) = slot {
                    calls.push(chunk);
                }
            }
        }
        Value::Null
        | Value::Bool(_)
        | Value::Number(_)
        | Value::Text(_)
        | Value::Bytes { .. }
        | Value::List(_)
        | Value::Tuple { .. } => {}
    }
}

fn to_json(value: &Value, field: &'static str) -> Result<Json, RepairError> {
    match value {
        Value::Null => Ok(Json::Null),
        Value::Bool(flag) => Ok(Json::Bool(*flag)),
        Value::Number(number) => Ok(Json::Number(number.clone())),
        Value::Text(text) => Ok(Json::Text(text.clone())),
        Value::List(items) | Value::Tuple { items } => Ok(Json::List(
            items
                .iter()
                .map(|item| to_json(item, field))
                .collect::<Result<_, _>>()?,
        )),
        Value::Dict(map) => Ok(Json::Object(
            map.iter()
                .map(|(key, item)| Ok((key.clone(), to_json(item, field)?)))
                .collect::<Result<_, _>>()?,
        )),
        Value::Bytes { .. } | Value::Instance { .. } => Err(RepairError::WrongType { field }),
    }
}
