pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let source = serde_json::to_string(input).map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "Serialize".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
        let raw = serde_json::from_str::<Box<serde_json::value::RawValue>>(&source).map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
        let parsed = betterlite_models::price_fields::PriceFields::parse(raw.as_ref()).map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
        let mut values = serde_json::Map::new();
        for group in parsed.values.values() {
            for (name, value) in group {
                values.insert(name.clone(), serde_json::Value::String(value.to_string()));
            }
        }
        let mut metadata = serde_json::Map::new();
        if let Some(value) = parsed.metadata.per_hour {
            metadata.insert("original_pricing_per_hour".to_owned(), serde_json::Value::String(value.to_string()));
        }
        if let Some(value) = parsed.metadata.per_minute {
            metadata.insert("original_pricing_per_minute".to_owned(), serde_json::Value::String(value.to_string()));
        }
        let mut sample = serde_json::Map::new();
        if let Some(value) = parsed.sample_spec.computer_use_input {
            sample.insert("computer_use_input_cost_per_1k_tokens".to_owned(), serde_json::Value::String(value.to_string()));
        }
        if let Some(value) = parsed.sample_spec.file_search_calls {
            sample.insert("file_search_cost_per_1k_calls".to_owned(), serde_json::Value::String(value.to_string()));
        }
        if let Some(value) = parsed.sample_spec.file_search_storage {
            sample.insert("file_search_cost_per_gb_per_day".to_owned(), serde_json::Value::String(value.to_string()));
        }
        Ok(serde_json::json!({
            "values": values,
            "provider_specific_us": parsed.provider_specific.us.map(|value| value.to_string()),
            "provider_specific_fast": parsed.provider_specific.fast.map(|value| value.to_string()),
            "guardrail_units": parsed.guardrail_units.iter().map(|(key, value)| (key.clone(), serde_json::Value::String(value.to_string()))).collect::<serde_json::Map<_, _>>(),
            "metadata": metadata,
            "sample_spec": sample,
            "off_peak": parsed.off_peak.map(|value| value.values.into_iter().map(|(key, price)| (key, serde_json::Value::String(price.to_string()))).collect::<serde_json::Map<_, _>>()),
            "search_context": parsed.search_context.into_iter().map(|(key, price)| (key, serde_json::Value::String(price.to_string()))).collect::<serde_json::Map<_, _>>(),
            "tiers": parsed.tiers.len(),
        }))
    }
}
