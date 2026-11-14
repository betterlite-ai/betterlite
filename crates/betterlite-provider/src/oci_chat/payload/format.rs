use betterlite_prompts::arguments::json::{Json, Object};

use crate::oci_chat_wire::types::oci::OciVendor;
use crate::oci_common::OciError;

const MISSING_FUNCTION_NAME: &str =
    "Invalid tool_choice for OCI: 'FUNCTION' type requires a non-empty function name";

const OK_TOOL_CHOICE_TYPES: [&str; 3] = ["AUTO", "NONE", "REQUIRED"];

pub(super) fn normalize_tool_choice(choice: &Json) -> Result<Json, OciError> {
    match choice {
        Json::Null => Ok(Json::Null),
        Json::Text(text) => Ok(tool_choice_from_string(text)),
        Json::Object(fields) => tool_choice_from_object(fields),
        Json::Bool(_) | Json::Number(_) | Json::List(_) => Err(OciError {
            status: 400,
            message: format!(
                "Invalid tool_choice for OCI: expected str or dict, got {}",
                json_kind(choice)
            ),
        }),
    }
}

fn tool_choice_from_string(text: &str) -> Json {
    match text.to_lowercase().as_str() {
        "auto" => typed_object("AUTO", None),
        "none" => typed_object("NONE", None),
        "required" | "any" => typed_object("REQUIRED", None),
        _ => typed_object("FUNCTION", Some(text)),
    }
}

fn tool_choice_from_object(fields: &Object) -> Result<Json, OciError> {
    let Some(Json::Text(raw_type)) = fields.get("type") else {
        return Err(OciError {
            status: 400,
            message: format!(
                "Invalid tool_choice for OCI: missing or non-string 'type' in {}",
                serde_json::to_string(fields).unwrap_or_default()
            ),
        });
    };
    let upper = raw_type.to_uppercase();
    if upper == "FUNCTION" {
        let found = match fields.get("function") {
            Some(Json::Object(function)) => function.get("name"),
            _ => fields.get("name"),
        };
        return match found {
            Some(Json::Text(name)) if !name.is_empty() => Ok(typed_object("FUNCTION", Some(name))),
            _ => Err(OciError {
                status: 400,
                message: MISSING_FUNCTION_NAME.to_owned(),
            }),
        };
    }
    if OK_TOOL_CHOICE_TYPES.contains(&upper.as_str()) {
        return Ok(typed_object(&upper, None));
    }
    Err(OciError {
        status: 400,
        message: format!(
            "Invalid tool_choice for OCI: unsupported type {raw_type}; expected one of 'FUNCTION', 'AUTO', 'NONE', 'REQUIRED'"
        ),
    })
}

fn json_kind(value: &Json) -> &'static str {
    match value {
        Json::Null => "NoneType",
        Json::Bool(_) => "bool",
        Json::Number(number) if number.is_f64() => "float",
        Json::Number(_) => "int",
        Json::Text(_) => "str",
        Json::List(_) => "list",
        Json::Object(_) => "dict",
    }
}

pub(super) fn normalize_response_format(
    current: &Json,
    vendor: OciVendor,
) -> Result<Json, OciError> {
    let Json::Object(format) = current else {
        return Ok(current.clone());
    };
    let Some(type_value) = format.get("type") else {
        return Ok(current.clone());
    };
    let Json::Text(raw_type) = type_value else {
        return Err(OciError {
            status: 400,
            message: String::from("response_format type must be a string"),
        });
    };
    let format_type = raw_type.to_lowercase();
    if format_type == "text" {
        return Ok(typed_object("TEXT", None));
    }
    if vendor == OciVendor::Cohere {
        return Ok(cohere_response_format(format));
    }
    if format_type == "json_schema" {
        return json_schema_format(format);
    }
    let upper = format_type.to_uppercase();
    let kind = if upper == "JSON" {
        "JSON_OBJECT"
    } else {
        &upper
    };
    Ok(typed_object(kind, None))
}

fn cohere_response_format(format: &Object) -> Json {
    let mut payload = Object::new();
    payload.insert(
        String::from("type"),
        Json::Text(String::from("JSON_OBJECT")),
    );
    if let Some(schema) = format
        .get("json_schema")
        .and_then(Json::as_object)
        .and_then(|json_schema| json_schema.get("schema"))
        .filter(|value| !matches!(value, Json::Null))
    {
        payload.insert(String::from("schema"), schema.clone());
    }
    Json::Object(payload)
}

fn json_schema_format(format: &Object) -> Result<Json, OciError> {
    let Some(json_schema) = format.get("json_schema").and_then(Json::as_object) else {
        return Err(OciError {
            status: 400,
            message: String::from(
                "response_format type 'json_schema' requires a 'json_schema' object",
            ),
        });
    };
    let present = |key: &str| {
        json_schema
            .get(key)
            .filter(|value| !matches!(value, Json::Null))
    };
    let mut retained = Object::new();
    retained.insert(
        String::from("name"),
        present("name")
            .cloned()
            .unwrap_or_else(|| Json::Text(String::from("response"))),
    );
    for (source, target) in [
        ("description", "description"),
        ("schema", "schema"),
        ("strict", "isStrict"),
    ] {
        if let Some(value) = present(source) {
            retained.insert(target.to_owned(), value.clone());
        }
    }
    let mut wrapper = Object::new();
    wrapper.insert(
        String::from("type"),
        Json::Text(String::from("JSON_SCHEMA")),
    );
    wrapper.insert(String::from("jsonSchema"), Json::Object(retained));
    Ok(Json::Object(wrapper))
}

fn typed_object(kind: &str, name: Option<&str>) -> Json {
    let mut fields = Object::new();
    fields.insert(String::from("type"), Json::Text(kind.to_owned()));
    if let Some(function) = name {
        fields.insert(String::from("name"), Json::Text(function.to_owned()));
    }
    Json::Object(fields)
}
