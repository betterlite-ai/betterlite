use betterlite_models::price_fields::PriceFields;
use serde_json::{Map, Value};

pub(crate) struct Op;

fn parse_input(input: &Value) -> Result<PriceFields, Box<crate::helpers::HelperError>> {
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
    let raw =
        serde_json::from_str::<Box<serde_json::value::RawValue>>(&source).map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
    PriceFields::parse(raw.as_ref()).map_err(|error| {
        Box::new(crate::helpers::HelperError {
            class: "InvalidInput".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })
}

fn prices(parsed: &PriceFields) -> Map<String, Value> {
    let mut values = Map::new();
    for group in parsed.values.values() {
        for (name, value) in group {
            values.insert(name.clone(), Value::String(value.to_string()));
        }
    }
    values
}

fn metadata(parsed: &PriceFields) -> Map<String, Value> {
    let mut values = Map::new();
    if let Some(value) = parsed.metadata.per_hour {
        values.insert(
            "original_pricing_per_hour".to_owned(),
            Value::String(value.to_string()),
        );
    }
    if let Some(value) = parsed.metadata.per_minute {
        values.insert(
            "original_pricing_per_minute".to_owned(),
            Value::String(value.to_string()),
        );
    }
    values
}

fn sample(parsed: &PriceFields) -> Map<String, Value> {
    let mut values = Map::new();
    if let Some(value) = parsed.sample_spec.computer_use_input {
        values.insert(
            "computer_use_input_cost_per_1k_tokens".to_owned(),
            Value::String(value.to_string()),
        );
    }
    if let Some(value) = parsed.sample_spec.file_search_calls {
        values.insert(
            "file_search_cost_per_1k_calls".to_owned(),
            Value::String(value.to_string()),
        );
    }
    if let Some(value) = parsed.sample_spec.file_search_storage {
        values.insert(
            "file_search_cost_per_gb_per_day".to_owned(),
            Value::String(value.to_string()),
        );
    }
    values
}

fn decimal_map<T>(values: impl IntoIterator<Item = (String, T)>) -> Map<String, Value>
where
    T: ToString,
{
    let mut result = Map::new();
    for (key, value) in values {
        result.insert(key, Value::String(value.to_string()));
    }
    result
}

fn project(parsed: PriceFields) -> Value {
    let values = prices(&parsed);
    let metadata = metadata(&parsed);
    let sample = sample(&parsed);
    let guardrail_units = decimal_map(parsed.guardrail_units.clone());
    let off_peak = parsed.off_peak.map(|value| decimal_map(value.values));
    let search_context = decimal_map(parsed.search_context.clone());
    serde_json::json!({
        "values": values,
        "provider_specific_us": parsed.provider_specific.us.map(|value| value.to_string()),
        "provider_specific_fast": parsed.provider_specific.fast.map(|value| value.to_string()),
        "guardrail_units": guardrail_units,
        "metadata": metadata,
        "sample_spec": sample,
        "off_peak": off_peak,
        "search_context": search_context,
        "tiers": parsed.tiers.len(),
    })
}

impl crate::helpers::Helper for Op {
    type Input = Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let parsed = parse_input(input)?;
        Ok(project(parsed))
    }
}
