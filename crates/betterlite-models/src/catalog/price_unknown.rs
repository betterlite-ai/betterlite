use std::collections::BTreeMap;

use serde_json::value::{RawValue, to_raw_value};

use super::CatalogError;

// price_fields/nested.rs is outside this task's scope, so these mirror its wire names directly.
const PROVIDER_SPECIFIC_FIELDS: &[&str] = &["us", "fast"];
const OFF_PEAK_FIELDS: &[&str] = &[
    "input_cost_per_token",
    "output_cost_per_token",
    "cache_read_input_token_cost",
    "cache_creation_input_token_cost",
    "output_cost_per_reasoning_token",
    "windows",
    "hours_utc",
    "weekday_timezone",
];
const PRICE_WINDOW_FIELDS: &[&str] = &["hours_utc", "weekdays"];
const SEARCH_CONTEXT_FIELDS: &[&str] = &[
    "search_context_size_low",
    "search_context_size_medium",
    "search_context_size_high",
];
const TIER_FIELDS: &[&str] = &[
    "input_cost_per_token",
    "output_cost_per_token",
    "output_cost_per_reasoning_token",
    "cache_read_input_token_cost",
    "input_cost_per_query",
    "cache_creation_input_token_cost",
    "range",
    "max_results_range",
];

type RawObject = BTreeMap<String, Box<RawValue>>;
type RawArray = Vec<Box<RawValue>>;

fn unknown_keys(
    value: &RawValue,
    known: &[&str],
    prefix: &str,
    paths: &mut Vec<String>,
) -> Result<(), CatalogError> {
    let fields: RawObject = serde_json::from_str(value.get())?;
    for key in fields.into_keys() {
        if !known.contains(&key.as_str()) {
            paths.push(format!("{prefix}.{key}"));
        }
    }
    Ok(())
}

// PriceFields decodes these compound fields whole, so a nested unknown field needs its own walk.
pub(super) fn unknown_paths(input: &RawValue) -> Result<Vec<String>, CatalogError> {
    let top_level: RawObject = serde_json::from_str(input.get())?;
    let mut paths = Vec::new();
    for (key, value) in &top_level {
        match key.as_str() {
            "provider_specific_entry" if value.get() != "null" => unknown_keys(
                value,
                PROVIDER_SPECIFIC_FIELDS,
                "provider_specific_entry",
                &mut paths,
            )?,
            "off_peak_pricing" if value.get() != "null" => off_peak_paths(value, &mut paths)?,
            "search_context_cost_per_query" if value.get() != "null" => unknown_keys(
                value,
                SEARCH_CONTEXT_FIELDS,
                "search_context_cost_per_query",
                &mut paths,
            )?,
            "tiered_pricing" => {
                let tiers: RawArray = serde_json::from_str(value.get())?;
                for (index, tier) in tiers.iter().enumerate() {
                    let prefix = format!("tiered_pricing[{index}]");
                    unknown_keys(tier, TIER_FIELDS, &prefix, &mut paths)?;
                }
            }
            _ => {}
        }
    }
    Ok(paths)
}

fn off_peak_paths(value: &RawValue, paths: &mut Vec<String>) -> Result<(), CatalogError> {
    unknown_keys(value, OFF_PEAK_FIELDS, "off_peak_pricing", paths)?;
    let fields: RawObject = serde_json::from_str(value.get())?;
    if let Some(raw_windows) = fields.get("windows") {
        let windows: RawArray = serde_json::from_str(raw_windows.get())?;
        for (index, window) in windows.iter().enumerate() {
            let prefix = format!("off_peak_pricing.windows[{index}]");
            unknown_keys(window, PRICE_WINDOW_FIELDS, &prefix, paths)?;
        }
    }
    Ok(())
}

// Warn mode reports the paths above, then drops them here so PriceFields can decode the rest.
pub(super) fn strip_unknown(input: &RawValue) -> Result<Box<RawValue>, CatalogError> {
    let mut fields: RawObject = serde_json::from_str(input.get()).unwrap_or_default();
    if let Some(value) = fields.get("provider_specific_entry")
        && value.get() != "null"
    {
        let kept = strip_keys(value, PROVIDER_SPECIFIC_FIELDS)?;
        fields.insert("provider_specific_entry".to_owned(), kept);
    }
    if let Some(value) = fields.get("off_peak_pricing")
        && value.get() != "null"
    {
        let kept = strip_off_peak(value)?;
        fields.insert("off_peak_pricing".to_owned(), kept);
    }
    if let Some(value) = fields.get("search_context_cost_per_query")
        && value.get() != "null"
    {
        let kept = strip_keys(value, SEARCH_CONTEXT_FIELDS)?;
        fields.insert("search_context_cost_per_query".to_owned(), kept);
    }
    if let Some(value) = fields.get("tiered_pricing") {
        let kept = strip_array(value, TIER_FIELDS)?;
        fields.insert("tiered_pricing".to_owned(), kept);
    }
    Ok(to_raw_value(&fields)?)
}

fn strip_keys(value: &RawValue, known: &[&str]) -> Result<Box<RawValue>, CatalogError> {
    let mut fields: BTreeMap<String, Box<RawValue>> = serde_json::from_str(value.get())?;
    fields.retain(|key, _| known.contains(&key.as_str()));
    Ok(to_raw_value(&fields)?)
}

fn strip_off_peak(value: &RawValue) -> Result<Box<RawValue>, CatalogError> {
    let filtered = strip_keys(value, OFF_PEAK_FIELDS)?;
    let mut fields: RawObject = serde_json::from_str(filtered.get()).unwrap_or_default();
    if let Some(raw_windows) = fields.get("windows") {
        let windows_raw = strip_array(raw_windows, PRICE_WINDOW_FIELDS)?;
        fields.insert("windows".to_owned(), windows_raw);
    }
    Ok(to_raw_value(&fields)?)
}

fn strip_array(value: &RawValue, known: &[&str]) -> Result<Box<RawValue>, CatalogError> {
    let items: RawArray = serde_json::from_str(value.get())?;
    let mut kept = Vec::with_capacity(items.len());
    for item in &items {
        kept.push(strip_keys(item, known)?);
    }
    Ok(to_raw_value(&kept)?)
}
