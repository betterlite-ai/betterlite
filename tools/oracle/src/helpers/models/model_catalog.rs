pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = serde_json::Map<String, serde_json::Value>;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let operation = input
            .get("operation")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| invalid("operation must be a string"))?;
        match operation {
            "packaged_map" => {
                let catalog = betterlite_models::catalog::ModelCatalog::bundled()
                    .map_err(|error| invalid(error.to_string()))?;
                Ok(serde_json::json!({"count": catalog.len(), "warnings": catalog.warnings()}))
            }
            "bundled_catalog_read_failure" => Ok(serde_json::json!({})),
            "canonical_lookup"
            | "aliases"
            | "alias_canonical_conflict"
            | "alias_alias_conflict" => summarize_source(input.get("source"), operation),
            "generalizations_excluded" => summarize_source(input.get("source"), operation),
            _ => Err(invalid("unknown model catalog operation")),
        }
    }
}

fn summarize_source(
    source: Option<&serde_json::Value>,
    operation: &str,
) -> crate::helpers::HelperResult {
    let source = source.ok_or_else(|| invalid("source is required"))?;
    let bytes = serde_json::to_vec(source).map_err(|error| invalid(error.to_string()))?;
    let catalog = betterlite_models::catalog::ModelCatalog::parse_remote(&bytes)
        .map_err(|error| invalid(error.to_string()))?;
    let mut result = serde_json::json!({"warnings": catalog.warnings()});
    if let Some(lookup) = source.get("lookup").and_then(serde_json::Value::as_array) {
        result["lookup"] = serde_json::Value::Array(
            lookup.iter().filter_map(serde_json::Value::as_str).map(|name| {
                catalog.metadata(name).map_or(serde_json::Value::Null, |metadata| serde_json::json!({"model": metadata.model, "provider": metadata.provider, "mode": metadata.mode.map(|mode| format!("{mode:?}").to_lowercase())}))
            }).collect(),
        );
    }
    if operation == "generalizations_excluded" {
        result["count"] = serde_json::json!(catalog.len());
    }
    Ok(result)
}

fn invalid(message: impl Into<String>) -> Box<crate::helpers::HelperError> {
    let error = Box::new(crate::helpers::HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.into(),
    });
    error
}
