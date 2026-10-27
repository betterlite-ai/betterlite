pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let mut document = input
            .get("openapi")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let function = input
            .get("function")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| error("function is required"))?;
        let result = match function {
            "add_schema_to_components" => {
                let name = input
                    .get("schema_name")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| error("schema_name is required"))?;
                let schema = input
                    .get("schema")
                    .cloned()
                    .ok_or_else(|| error("schema is required"))?;
                betterlite_openapi::schema_transforms::add_schema_to_components(
                    &mut document,
                    name,
                    schema,
                )
                .map(|()| document)
            }
            "add_request_body_to_paths" => {
                let paths = input
                    .get("paths")
                    .and_then(serde_json::Value::as_array)
                    .ok_or_else(|| error("paths is required"))?
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                let reference = input
                    .get("schema_ref")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| error("schema_ref is required"))?;
                betterlite_openapi::schema_transforms::add_request_body_to_paths(
                    &mut document, &paths, reference,
                )
                .map(|()| document)
            }
            "inline_request_body" => betterlite_openapi::inline_expansion::inline_request_body(
                input.get("schema").cloned(),
                input
                    .get("example")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null),
            ),
            _ => Err(
                betterlite_openapi::schema_transforms::SchemaError::InvalidComponent(
                    function.to_owned(),
                ),
            ),
        };
        result.map_err(|source| {
            Box::new(crate::helpers::HelperError {
                class: "SchemaError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: source.to_string(),
            })
        })
    }
}

fn error(message: &str) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
