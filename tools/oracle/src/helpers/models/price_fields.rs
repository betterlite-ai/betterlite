pub(crate) struct Op;

fn helper_error(class: &str, message: String) -> Box<crate::helpers::HelperError> {
    let error = crate::helpers::HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    };
    Box::new(error)
}

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let source = serde_json::to_string(input)
            .map_err(|error| helper_error("Serialize", error.to_string()))?;
        let raw = serde_json::from_str::<Box<serde_json::value::RawValue>>(&source)
            .map_err(|error| helper_error("InvalidInput", error.to_string()))?;
        let parsed = betterlite_models::price_fields::PriceFields::parse(raw.as_ref())
            .map_err(|error| helper_error("InvalidInput", error.to_string()))?;
        let mut values = serde_json::Map::new();
        for group in parsed.values.values() {
            for (name, value) in group {
                values.insert(name.clone(), serde_json::Value::String(value.to_string()));
            }
        }
        Ok(serde_json::json!({
            "values": values,
            "provider_specific_us": parsed.provider_specific.us.map(|value| value.to_string()),
            "provider_specific_fast": parsed.provider_specific.fast.map(|value| value.to_string()),
            "guardrail_units": parsed.guardrail_units.iter().map(|(key, value)| (key.clone(), serde_json::Value::String(value.to_string()))).collect::<serde_json::Map<_, _>>(),
        }))
    }
}
