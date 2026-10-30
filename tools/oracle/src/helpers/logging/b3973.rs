pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let function = input
            .get("function")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| crate::helpers::HelperError {
                class: "InvalidInput",
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: "function is required".to_owned(),
            })?;
        match function {
            "uuid" => {
                let uuid = uuid::Uuid::parse_str(&betterlite_logging::create_uuid7()).map_err(
                    |error| crate::helpers::HelperError {
                        class: "Uuid",
                        status: None,
                        error_type: None,
                        param: None,
                        code: None,
                        message: error.to_string(),
                    },
                )?;
                Ok(serde_json::json!({"version": uuid.get_version_num(), "variant": 2}))
            }
            "config" => {
                let key = input
                    .get("key")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                let user = input.get("user").and_then(serde_json::Value::as_str);
                let environment = input
                    .get("environment")
                    .and_then(serde_json::Value::as_object)
                    .map(|map| {
                        map.iter()
                            .filter_map(|(key, value)| {
                                value.as_str().map(|value| (key.clone(), value.to_owned()))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let config = input
                    .get("config")
                    .and_then(serde_json::Value::as_object)
                    .map(|map| {
                        map.iter()
                            .filter_map(|(key, value)| {
                                value.as_str().map(|value| (key.clone(), value.to_owned()))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let default = input.get("default").and_then(serde_json::Value::as_str);
                Ok(
                    serde_json::json!({"value": betterlite_logging::get_config_variable(key, user, &environment, &config, default)}),
                )
            }
            "metadata" => {
                let empty = serde_json::Map::new();
                let auth = input
                    .get("auth")
                    .and_then(serde_json::Value::as_object)
                    .unwrap_or(&empty);
                let request = input
                    .get("request")
                    .and_then(serde_json::Value::as_object)
                    .unwrap_or(&empty);
                let requester = input
                    .get("requester")
                    .and_then(serde_json::Value::as_object)
                    .unwrap_or(&empty);
                let mut metadata = auth.clone();
                metadata.extend(request.clone());
                metadata.extend(requester.clone());
                Ok(serde_json::json!({"metadata": metadata}))
            }
            "payload" => {
                let empty = serde_json::Map::new();
                let response = input
                    .get("response")
                    .and_then(serde_json::Value::as_object)
                    .unwrap_or(&empty);
                let usage = betterlite_logging::create_usage_object(
                    response
                        .get("usage")
                        .and_then(serde_json::Value::as_object)
                        .unwrap_or(&empty),
                );
                Ok(
                    serde_json::json!({"type": "llm", "model": response.get("model").and_then(serde_json::Value::as_str).unwrap_or("unknown-model"), "usage": usage}),
                )
            }
            _ => Self::not_built("b3973"),
        }
    }
}
