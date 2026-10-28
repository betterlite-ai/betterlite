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
    let entries = parsed
        .values
        .values()
        .flat_map(|group| group.iter())
        .map(|(name, value)| (name.clone(), Value::String(value.to_string())));
    value_map(entries)
}

fn value_map(entries: impl Iterator<Item = (String, Value)>) -> Map<String, Value> {
    let mut values = Map::new();
    for (name, value) in entries {
        values.insert(name, value);
    }
    values
}

fn optional_values<T, const COUNT: usize>(entries: [(&str, Option<T>); COUNT]) -> Map<String, Value>
where
    T: ToString,
{
    let mut values = Map::new();
    for (name, option) in entries {
        if let Some(value) = option {
            values.insert(name.to_owned(), Value::String(value.to_string()));
        }
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
    let metadata = optional_values([
        ("original_pricing_per_hour", parsed.metadata.per_hour),
        ("original_pricing_per_minute", parsed.metadata.per_minute),
    ]);
    let guardrail_units = decimal_map(parsed.guardrail_units.clone());
    let off_peak = parsed.off_peak.map(|value| decimal_map(value.values));
    let search_context = decimal_map(parsed.search_context.clone());
    serde_json::json!({
        "values": values,
        "provider_specific_us": parsed.provider_specific.us.map(|value| value.to_string()),
        "provider_specific_fast": parsed.provider_specific.fast.map(|value| value.to_string()),
        "guardrail_units": guardrail_units,
        "metadata": metadata,
        "sample_spec": optional_values([
            ("computer_use_input_cost_per_1k_tokens", parsed.sample_spec.computer_use_input),
            ("file_search_cost_per_1k_calls", parsed.sample_spec.file_search_calls),
            ("file_search_cost_per_gb_per_day", parsed.sample_spec.file_search_storage),
        ]),
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
