use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::gigachat_chat::error::GigaChatError;
use crate::openai_base_config::Object;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 11] = [
    "stream",
    "temperature",
    "top_p",
    "max_tokens",
    "max_completion_tokens",
    "stop",
    "tools",
    "tool_choice",
    "functions",
    "function_call",
    "response_format",
];

pub const STRUCTURED_OUTPUT: &str = "_structured_output";

const DEFAULT_SCHEMA_NAME: &str = "structured_output";

const LITELLM_ORDER: [&str; 10] = [
    "functions",
    "function_call",
    "temperature",
    "top_p",
    "stream",
    "max_tokens",
    "max_completion_tokens",
    "response_format",
    "tools",
    "tool_choice",
];

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct FunctionTool {
    name: Option<String>,
    description: Option<String>,
    parameters: Option<Object>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct FunctionSchema {
    name: Option<String>,
    schema: Option<Object>,
}

#[derive(Serialize)]
struct Function {
    name: String,
    description: String,
    parameters: Object,
}

#[derive(Serialize)]
struct FunctionName<'name> {
    name: &'name str,
}

pub fn map_openai_params(
    non_default_params: &Object,
    mut optional_params: Object,
) -> Result<Object, GigaChatError> {
    for name in LITELLM_ORDER {
        let Some(value) = non_default_params
            .get(name)
            .filter(|value| !value.is_null())
        else {
            continue;
        };
        match name {
            "stream" | "top_p" | "functions" | "function_call" => {
                optional_params.insert(String::from(name), value.clone());
            }
            "temperature" => insert_temperature(value, &mut optional_params),
            "max_tokens" | "max_completion_tokens" => {
                optional_params.insert(String::from("max_tokens"), value.clone());
            }
            "tools" => {
                let functions = tools_to_functions(value)?;
                optional_params.insert(String::from("functions"), Value::Array(functions));
            }
            "tool_choice" => {
                if let Some(choice) = map_tool_choice(value) {
                    optional_params.insert(String::from("function_call"), choice);
                }
            }
            "response_format" => apply_response_format(value, &mut optional_params)?,
            _ => {}
        }
    }
    Ok(optional_params)
}

fn insert_temperature(temperature: &Value, optional_params: &mut Object) {
    if temperature
        .as_f64()
        .is_some_and(|magnitude| magnitude == 0.0_f64)
    {
        optional_params.insert(String::from("top_p"), Value::from(0_i32));
    } else {
        optional_params.insert(String::from("temperature"), temperature.clone());
    }
}

fn tools_to_functions(tools: &Value) -> Result<Vec<Value>, GigaChatError> {
    let Value::Array(entries) = tools else {
        return Err(GigaChatError {
            status_code: 400,
            message: String::from("tools must be a list of tool objects"),
            headers: None,
            source: None,
        });
    };
    let mut functions = Vec::with_capacity(entries.len());
    for entry in entries {
        let Some(tool) = entry.as_object() else {
            continue;
        };
        if tool.get("type").and_then(Value::as_str) != Some("function") {
            continue;
        }
        let function: FunctionTool = match tool.get("function") {
            None | Some(Value::Null) => FunctionTool::default(),
            Some(value) => {
                serde_json::from_value(value.clone()).map_err(|source| GigaChatError {
                    status_code: 400,
                    message: format!("a function tool holds an object function: {source}"),
                    headers: None,
                    source: Some(source),
                })?
            }
        };
        functions.push(json!(Function {
            name: function.name.unwrap_or_default(),
            description: function.description.unwrap_or_default(),
            parameters: function.parameters.unwrap_or_default(),
        }));
    }
    Ok(functions)
}

fn map_tool_choice(tool_choice: &Value) -> Option<Value> {
    match tool_choice {
        Value::String(choice) if choice == "none" || choice == "auto" => {
            Some(Value::String(choice.clone()))
        }
        Value::String(choice) if choice == "required" => Some(Value::String(String::from("auto"))),
        Value::Object(choice) => {
            if choice.get("type").and_then(Value::as_str) != Some("function") {
                return None;
            }
            let name = choice
                .get("function")
                .and_then(Value::as_object)
                .and_then(|implementation| implementation.get("name"))
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())?;
            Some(json!(FunctionName { name }))
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
            None
        }
    }
}

fn apply_response_format(
    format: &Value,
    optional_params: &mut Object,
) -> Result<(), GigaChatError> {
    let Value::Object(fields) = format else {
        return Err(GigaChatError {
            status_code: 400,
            message: String::from("response_format must be an object"),
            headers: None,
            source: None,
        });
    };
    if fields.get("type").and_then(Value::as_str) != Some("json_schema") {
        return Ok(());
    }
    let schema: FunctionSchema = match fields.get("json_schema") {
        None | Some(Value::Null) => FunctionSchema::default(),
        Some(value) => serde_json::from_value(value.clone()).map_err(|source| GigaChatError {
            status_code: 400,
            message: format!("response_format.json_schema holds declared fields: {source}"),
            headers: None,
            source: Some(source),
        })?,
    };
    let name = schema
        .name
        .unwrap_or_else(|| String::from(DEFAULT_SCHEMA_NAME));
    let mut functions = optional_params
        .get("functions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    functions.push(json!(Function {
        description: format!("Output structured response: {name}"),
        name: name.clone(),
        parameters: schema.schema.unwrap_or_default(),
    }));
    optional_params.insert(String::from("functions"), Value::Array(functions));
    let call = json!(FunctionName { name: &name });
    optional_params.insert(String::from("function_call"), call);
    optional_params.insert(String::from(STRUCTURED_OUTPUT), Value::Bool(true));
    Ok(())
}
