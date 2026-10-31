pub(crate) struct Op;

fn simple_error(class: &str, message: String) -> crate::helpers::HelperError {
    let owned_class = class.to_owned();
    crate::helpers::HelperError {
        class: owned_class,
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    }
}

fn object_field(
    value: &serde_json::Value,
    field: &str,
) -> serde_json::Map<String, serde_json::Value> {
    let Some(object) = value.get(field).and_then(serde_json::Value::as_object) else {
        return serde_json::Map::new();
    };
    object.clone()
}

fn string_map(
    value: &serde_json::Value,
    field: &str,
) -> std::collections::BTreeMap<String, String> {
    let object = object_field(value, field);
    object
        .iter()
        .filter_map(|(entry_key, entry_value)| {
            entry_value
                .as_str()
                .map(|text| (entry_key.clone(), text.to_owned()))
        })
        .collect()
}

fn run_uuid() -> crate::helpers::HelperResult {
    let created = betterlite_logging::opik::uuid7::create_uuid7();
    let uuid =
        uuid::Uuid::parse_str(&created).map_err(|error| simple_error("Uuid", error.to_string()))?;
    Ok(serde_json::json!({"version": uuid.get_version_num(), "variant": 2}))
}

fn run_config(input: &serde_json::Value) -> serde_json::Value {
    let key = input
        .get("key")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let user = input.get("user").and_then(serde_json::Value::as_str);
    let environment = string_map(input, "environment");
    let config = string_map(input, "config");
    let default = input.get("default").and_then(serde_json::Value::as_str);
    serde_json::json!({"value": betterlite_logging::opik::config::get_config_variable(key, user, &environment, &config, default)})
}

fn run_metadata(input: &serde_json::Value) -> serde_json::Value {
    let mut metadata = object_field(input, "auth");
    metadata.extend(object_field(input, "request"));
    metadata.extend(object_field(input, "requester"));
    serde_json::json!({"metadata": metadata})
}

fn run_payload(input: &serde_json::Value) -> serde_json::Value {
    let response = object_field(input, "response");
    let usage_value = response
        .get("usage")
        .cloned()
        .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));
    let usage_map: indexmap::IndexMap<String, betterlite_models::decisions::Json> =
        serde_json::from_value(usage_value).unwrap_or_default();
    let usage = betterlite_logging::opik::config::create_usage_object(&usage_map);
    let model = response
        .get("model")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown-model");
    serde_json::json!({"type": "llm", "model": model, "usage": usage})
}

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let function = input
            .get("function")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| simple_error("InvalidInput", "function is required".to_owned()))?;
        match function {
            "uuid" => run_uuid(),
            "config" => Ok(run_config(input)),
            "metadata" => Ok(run_metadata(input)),
            "payload" => Ok(run_payload(input)),
            _ => Self::not_built("b3973"),
        }
    }
}
