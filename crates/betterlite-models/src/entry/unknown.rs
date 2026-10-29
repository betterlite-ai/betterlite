use crate::capabilities::media::MediaCapabilities;
use crate::capabilities::reasoning::ReasoningCapabilities;
use crate::capabilities::tooling::ToolingCapabilities;
use crate::capabilities::vendor::VendorCapabilities;
use crate::metadata::MetadataFields;
use crate::metadata::notes::CatalogNotes;
use crate::price_fields::values::cache::CachePricing;
use crate::price_fields::values::input::InputPricing;
use crate::price_fields::values::other::OtherPricing;
use crate::price_fields::values::output::OutputPricing;
use serde_json::value::RawValue;
use std::collections::BTreeMap;

// These price field names stay recognized so an ordinary entry isn't reported as "unknown".
const PRICE_COMPOUND_NAMES: &[&str] = &[
    "provider_specific_entry",
    "guardrail_cost_per_unit",
    "off_peak_pricing",
    "search_context_cost_per_query",
    "tiered_pricing",
];
const PRICE_METADATA_NAMES: &[&str] = &["original_pricing_per_hour", "original_pricing_per_minute"];

const PRICE_FLAT_WIRE_NAME_GROUPS: &[&[&str]] = &[
    CachePricing::WIRE_NAMES,
    InputPricing::WIRE_NAMES,
    OutputPricing::WIRE_NAMES,
    OtherPricing::WIRE_NAMES,
];

pub(super) const FLAT_WIRE_NAME_GROUPS: &[&[&str]] = &[
    MetadataFields::WIRE_NAMES,
    ReasoningCapabilities::WIRE_NAMES,
    MediaCapabilities::WIRE_NAMES,
    ToolingCapabilities::WIRE_NAMES,
    VendorCapabilities::WIRE_NAMES,
];

fn parse_object(input: &RawValue) -> BTreeMap<String, Box<RawValue>> {
    let parsed = serde_json::from_str(input.get());
    parsed.unwrap_or_default()
}

// Reports a remote refresh's new field this way rather than merging or dropping it silently.
pub(super) fn unknown_paths(input: &RawValue, include_price_fields: bool) -> Vec<String> {
    let top_level = parse_object(input);
    let mut paths = Vec::new();
    for (key, value) in &top_level {
        match key.as_str() {
            "metadata" => paths.extend(unknown_note_keys(value, include_price_fields)),
            other if !is_known_top_level(other, include_price_fields) => paths.push(key.clone()),
            _ => {}
        }
    }
    paths
}

fn is_known_top_level(key: &str, include_price_fields: bool) -> bool {
    let in_model_fields = FLAT_WIRE_NAME_GROUPS
        .iter()
        .any(|group| group.contains(&key));
    in_model_fields
        || (include_price_fields
            && (PRICE_COMPOUND_NAMES.contains(&key)
                || PRICE_FLAT_WIRE_NAME_GROUPS
                    .iter()
                    .any(|group| group.contains(&key))))
}

fn is_known_note(key: &str, include_price_fields: bool) -> bool {
    let in_catalog_notes = CatalogNotes::WIRE_NAMES.contains(&key);
    in_catalog_notes || (include_price_fields && PRICE_METADATA_NAMES.contains(&key))
}

fn unknown_note_keys(value: &RawValue, include_price_fields: bool) -> Vec<String> {
    let notes = parse_object(value);
    notes
        .into_keys()
        .filter(|key| !is_known_note(key, include_price_fields))
        .map(|key| format!("metadata.{key}"))
        .collect()
}
