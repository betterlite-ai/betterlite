use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde::Deserialize;
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
        serde_json::from_str(value.get()).map_err(|source| PriceFieldError::NotObject {
            path: path.to_owned(),
            source,
        })?;
    Ok(object)
}

fn provider_specific(value: &RawValue) -> Result<ProviderSpecificPricing, PriceFieldError> {
    let object = object_fields(value, "provider_specific_entry")?;
    Ok(ProviderSpecificPricing {
        us: object
            .get("us")
            .map(|raw_value| decimal_at("provider_specific_entry.us", raw_value))
            .transpose()?,
        fast: object
            .get("fast")
            .map(|raw_value| decimal_at("provider_specific_entry.fast", raw_value))
            .transpose()?,
    })
}

fn metadata(value: &RawValue) -> Result<MetadataPricing, PriceFieldError> {
    let object = object_fields(value, "metadata")?;
    Ok(MetadataPricing {
        per_hour: object
            .get("original_pricing_per_hour")
            .map(|raw_value| decimal_at("metadata.original_pricing_per_hour", raw_value))
            .transpose()?,
        per_minute: object
            .get("original_pricing_per_minute")
            .map(|raw_value| decimal_at("metadata.original_pricing_per_minute", raw_value))
            .transpose()?,
    })
}

fn sample(value: &RawValue) -> Result<SamplePricing, PriceFieldError> {
    let object = object_fields(value, "sample_spec")?;
    let computer_use_input = object
        .get("computer_use_input_cost_per_1k_tokens")
        .map(|raw_value| {
            decimal_at(
                "sample_spec.computer_use_input_cost_per_1k_tokens",
                raw_value,
            )
        })
        .transpose()?;
    let computer_use_output = object
        .get("computer_use_output_cost_per_1k_tokens")
        .map(|raw_value| {
            decimal_at(
                "sample_spec.computer_use_output_cost_per_1k_tokens",
                raw_value,
            )
        })
        .transpose()?;
    let file_search_calls = object
        .get("file_search_cost_per_1k_calls")
        .map(|raw_value| decimal_at("sample_spec.file_search_cost_per_1k_calls", raw_value))
        .transpose()?;
    let file_search_storage = object
        .get("file_search_cost_per_gb_per_day")
        .map(|raw_value| decimal_at("sample_spec.file_search_cost_per_gb_per_day", raw_value))
        .transpose()?;
    let vector_store_storage = object
        .get("vector_store_cost_per_gb_per_day")
        .map(|raw_value| decimal_at("sample_spec.vector_store_cost_per_gb_per_day", raw_value))
        .transpose()?;
    Ok(SamplePricing {
        computer_use_input,
        computer_use_output,
        file_search_calls,
        file_search_storage,
        vector_store_storage,
    })
}

fn off_peak(value: &RawValue) -> Result<OffPeakPricing, PriceFieldError> {
    let raw_fields: BTreeMap<String, Box<RawValue>> =
        serde_json::from_str(value.get()).map_err(|source| PriceFieldError::NotObject {
            path: "off_peak_pricing".to_owned(),
            source,
        })?;
    for name in [
        "input_cost_per_token",
        "output_cost_per_token",
        "cache_read_input_token_cost",
    ] {
        if let Some(raw) = raw_fields.get(name) {
            let path = format!("off_peak_pricing.{name}");
            decimal_at(&path, raw.as_ref())?;
        }
    }
    let object: OffPeakWire =
        serde_json::from_str(value.get()).map_err(|source| PriceFieldError::NotObject {
            path: "off_peak_pricing".to_owned(),
            source,
        })?;
    let mut values = BTreeMap::new();
    if let Some(price) = object.input_cost_per_token {
        values.insert("input_cost_per_token".to_owned(), price);
    }
    if let Some(price) = object.output_cost_per_token {
        values.insert("output_cost_per_token".to_owned(), price);
    }
    if let Some(price) = object.cache_read_input_token_cost {
        values.insert("cache_read_input_token_cost".to_owned(), price);
    }
    Ok(OffPeakPricing {
        values,
        windows: object.windows,
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OffPeakWire {
    input_cost_per_token: Option<Decimal>,
    output_cost_per_token: Option<Decimal>,
    cache_read_input_token_cost: Option<Decimal>,
    #[serde(rename = "hours_utc")]
    _hours_utc: Option<HoursUtc>,
    windows: Option<Vec<PriceWindow>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(untagged)]
enum HoursUtc {
    One(String),
    Many(Vec<String>),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct PriceWindow {
    hours_utc: HoursUtc,
    weekdays: Vec<Weekday>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(untagged)]
enum Weekday {
    Name(String),
    Number(u8),
}

fn tiers(value: &RawValue) -> Result<Vec<TierPricing>, PriceFieldError> {
    let entries: Vec<BTreeMap<String, Box<RawValue>>> =
        serde_json::from_str(value.get()).map_err(|source| PriceFieldError::NotArray {
            path: "tiered_pricing".to_owned(),
            source,
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
                        range = parse_bounds(&path, raw.as_ref())?;
                    }
                    _ if PriceField::from_wire(&name).is_some() => {
                        values.insert(name, decimal_at(&path, raw.as_ref())?);
                    }
                    "max_results_range" => {
                        range = parse_bounds(&path, raw.as_ref())?;
                    }
                    _ => return Err(PriceFieldError::UnknownTier { path }),
                }
            }
            Ok(TierPricing { values, range })
        })
        .collect()
}

fn parse_bounds(path: &str, value: &RawValue) -> Result<Vec<Decimal>, PriceFieldError> {
    let bounds: Vec<Box<RawValue>> =
        serde_json::from_str(value.get()).map_err(|source| PriceFieldError::NotArray {
            path: path.to_owned(),
            source,
        })?;
    bounds
        .into_iter()
        .map(|bound| decimal_at(path, bound.as_ref()))
        .collect()
}
