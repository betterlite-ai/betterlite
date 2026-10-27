use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde_json::value::RawValue;

use super::{
    MetadataPricing, OffPeakPricing, PriceField, PriceFieldError, PriceFields,
    ProviderSpecificPricing, SamplePricing, TierPricing, decimal_at, decimal_map,
};

pub(super) fn apply_nested(
    parsed: &mut PriceFields,
    name: &str,
    value: &RawValue,
) -> Result<(), PriceFieldError> {
    match name {
        "guardrail_cost_per_unit" => parsed.guardrail_units = decimal_map(name, value)?,
        "provider_specific_entry" => parsed.provider_specific = provider_specific(value)?,
        "metadata" => parsed.metadata = metadata(value)?,
        "sample_spec" => parsed.sample_spec = sample(value)?,
        "off_peak_pricing" => parsed.off_peak = Some(off_peak(value)?),
        "search_context_cost_per_query" => parsed.search_context = decimal_map(name, value)?,
        "tiered_pricing" => parsed.tiers = tiers(value)?,
        _ => {}
    }
    Ok(())
}

fn object_fields(
    value: &RawValue,
    path: &str,
) -> Result<BTreeMap<String, Box<RawValue>>, PriceFieldError> {
    let object =
        serde_json::from_str(value.get()).map_err(|_error| PriceFieldError::NotNumber {
            path: path.to_owned(),
        })?;
    Ok(object)
}

fn optional_decimal(
    object: &BTreeMap<String, Box<RawValue>>,
    name: &str,
    path: &str,
) -> Result<Option<Decimal>, PriceFieldError> {
    let parsed = object
        .get(name)
        .map(|raw_value| decimal_at(path, raw_value))
        .transpose()?;
    Ok(parsed)
}

fn provider_specific(value: &RawValue) -> Result<ProviderSpecificPricing, PriceFieldError> {
    let object = object_fields(value, "provider_specific_entry")?;
    Ok(ProviderSpecificPricing {
        us: optional_decimal(&object, "us", "provider_specific_entry.us")?,
        fast: optional_decimal(&object, "fast", "provider_specific_entry.fast")?,
    })
}

fn metadata(value: &RawValue) -> Result<MetadataPricing, PriceFieldError> {
    let object = object_fields(value, "metadata")?;
    Ok(MetadataPricing {
        per_hour: optional_decimal(
            &object,
            "original_pricing_per_hour",
            "metadata.original_pricing_per_hour",
        )?,
        per_minute: optional_decimal(
            &object,
            "original_pricing_per_minute",
            "metadata.original_pricing_per_minute",
        )?,
    })
}

fn sample(value: &RawValue) -> Result<SamplePricing, PriceFieldError> {
    let object = object_fields(value, "sample_spec")?;
    let get = |name: &str| optional_decimal(&object, name, &format!("sample_spec.{name}"));
    let computer_use_input = get("computer_use_input_cost_per_1k_tokens")?;
    let computer_use_output = get("computer_use_output_cost_per_1k_tokens")?;
    let file_search_calls = get("file_search_cost_per_1k_calls")?;
    let file_search_storage = get("file_search_cost_per_gb_per_day")?;
    let vector_store_storage = get("vector_store_cost_per_gb_per_day")?;
    Ok(SamplePricing {
        computer_use_input,
        computer_use_output,
        file_search_calls,
        file_search_storage,
        vector_store_storage,
    })
}

fn off_peak(value: &RawValue) -> Result<OffPeakPricing, PriceFieldError> {
    let values = decimal_map("off_peak_pricing", value)?;
    Ok(OffPeakPricing { values })
}

fn tiers(value: &RawValue) -> Result<Vec<TierPricing>, PriceFieldError> {
    let entries: Vec<BTreeMap<String, Box<RawValue>>> =
        serde_json::from_str(value.get()).map_err(|_error| PriceFieldError::NotNumber {
            path: "tiered_pricing".to_owned(),
        })?;
    entries
        .into_iter()
        .enumerate()
        .map(|(index, entry)| {
            let mut values = BTreeMap::new();
            let mut range = Vec::new();
            for (name, raw) in entry {
                let path = format!("tiered_pricing[{index}].{name}");
                match name.as_str() {
                    "range" => {
                        let bounds: Vec<Box<RawValue>> = serde_json::from_str(raw.get())
                            .map_err(|_error| PriceFieldError::NotNumber { path: path.clone() })?;
                        range = bounds
                            .into_iter()
                            .map(|bound| decimal_at(&path, bound.as_ref()))
                            .collect::<Result<_, _>>()?;
                    }
                    _ if PriceField::from_wire(&name).is_some() => {
                        values.insert(name, decimal_at(&path, raw.as_ref())?);
                    }
                    _ => {}
                }
            }
            Ok(TierPricing { values, range })
        })
        .collect()
}
