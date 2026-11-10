use serde_json::Value;

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

// LiteLLM marks an emulated structured-output request with this option, and never sends it to GigaChat
pub const STRUCTURED_OUTPUT: &str = "_structured_output";

const DEFAULT_SCHEMA_NAME: &str = "structured_output";

pub fn map_openai_params(
    non_default_params: &Object,
    mut optional_params: Object,
) -> Result<Object, GigaChatError> {
    for (name, value) in non_default_params {
        // the project reads a null parameter as an absent one
        if value.is_null() {
            continue;
        }
        match name.as_str() {
            "stream" => {
                optional_params.insert(String::from("stream"), value.clone());
            }
            "temperature" => insert_temperature(value, &mut optional_params),
            "top_p" => {
                optional_params.insert(String::from("top_p"), value.clone());
            }
            "max_tokens" | "max_completion_tokens" => {
                optional_params.insert(String::from("max_tokens"), value.clone());
            }
            "tools" => {
                optional_params.insert(
                    String::from("functions"),
                    Value::Array(tools_to_functions(value)?),
                );
            }
            "tool_choice" => {
                if let Some(choice) = map_tool_choice(value) {
                    optional_params.insert(String::from("function_call"), choice);
                }
            }
            "functions" | "function_call" => {
                optional_params.insert(name.clone(), value.clone());
            }
            "response_format" => apply_response_format(value, &mut optional_params)?,
            // LiteLLM accepts "stop" and sends nothing, and drops every option GigaChat does not take
            _ => {}
        }
    }
    Ok(optional_params)
}

// GigaChat answers a temperature of zero with top_p zero instead, and no temperature at all
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

// LiteLLM keeps only function tools and writes the three function fields with empty defaults
fn tools_to_functions(tools: &Value) -> Result<Vec<Value>, GigaChatError> {
    let Value::Array(entries) = tools else {
        return Err(GigaChatError {
            status_code: 400,
            message: String::from("tools must be a list of tool objects"),
            headers: None,
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
        let definition = tool.get("function").and_then(Value::as_object);
        let field = |name: &str, default: Value| {
            definition
                .and_then(|found| found.get(name))
                .cloned()
                .unwrap_or_else(|| default.clone())
        };
        functions.push(Value::Object(Object::from_iter([
            (
                String::from("name"),
                field("name", Value::String(String::new())),
            ),
            (
                String::from("description"),
                field("description", Value::String(String::new())),
            ),
            (
                String::from("parameters"),
                field("parameters", Value::Object(Object::new())),
            ),
        ])));
    }
    Ok(functions)
}

// LiteLLM maps the three OpenAI modes, sends a forced function as its name alone and drops anything else
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
            Some(Value::Object(Object::from_iter([(
                String::from("name"),
                Value::String(name.to_owned()),
            )])))
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
            None
        }
    }
}

// LiteLLM emulates a json_schema response format with a function whose schema GigaChat can call
fn apply_response_format(
    format: &Value,
    optional_params: &mut Object,
) -> Result<(), GigaChatError> {
    let Value::Object(fields) = format else {
        return Err(GigaChatError {
            status_code: 400,
            message: String::from("response_format must be an object"),
            headers: None,
        });
    };
    if fields.get("type").and_then(Value::as_str) != Some("json_schema") {
        return Ok(());
    }
    let (name, parameters) = json_schema_fields(fields)?;
    let function = Object::from_iter([
        (String::from("name"), Value::String(name.clone())),
        (
            String::from("description"),
            Value::String(format!("Output structured response: {name}")),
        ),
        (String::from("parameters"), parameters),
    ]);
    let mut functions = optional_params
        .get("functions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    functions.push(Value::Object(function));
    optional_params.insert(String::from("functions"), Value::Array(functions));
    optional_params.insert(
        String::from("function_call"),
        Value::Object(Object::from_iter([(
            String::from("name"),
            Value::String(name),
        )])),
    );
    optional_params.insert(String::from(STRUCTURED_OUTPUT), Value::Bool(true));
    Ok(())
}

fn json_schema_fields(format: &Object) -> Result<(String, Value), GigaChatError> {
    let json_schema = match format.get("json_schema") {
        None | Some(Value::Null) => Object::new(),
        Some(Value::Object(schema)) => schema.clone(),
        Some(Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_)) => {
            return Err(GigaChatError {
                status_code: 400,
                message: String::from("response_format.json_schema must be an object"),
                headers: None,
            });
        }
    };
    let name = match json_schema.get("name") {
        None | Some(Value::Null) => String::from(DEFAULT_SCHEMA_NAME),
        Some(Value::String(name)) => name.clone(),
        Some(Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_)) => {
            return Err(GigaChatError {
                status_code: 400,
                message: String::from("response_format.json_schema.name must be a string"),
                headers: None,
            });
        }
    };
    let parameters = match json_schema.get("schema") {
        None | Some(Value::Null) => Value::Object(Object::new()),
        Some(schema @ Value::Object(_)) => schema.clone(),
        Some(Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_)) => {
            return Err(GigaChatError {
                status_code: 400,
                message: String::from("response_format.json_schema.schema must be an object"),
                headers: None,
            });
        }
    };
    Ok((name, parameters))
}
