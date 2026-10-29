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

// Catalog entries also carry B204's price fields, which ModelFields does not
// model. These names stay recognized so an ordinary pinned entry does not
// read as full of "unknown" fields; only a genuinely new key does.
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

/// Lists the complete path of every top-level or `metadata`-nested field a catalog entry carries
/// that neither `ModelFields` nor B204's price fields recognize.
///
/// A remote refresh's new field is reported this way rather than merged with a known one or
/// dropped silently.
pub(super) fn unknown_paths(input: &RawValue, include_price_fields: bool) -> Vec<String> {
    let Ok(top_level) = serde_json::from_str::<BTreeMap<String, Box<RawValue>>>(input.get()) else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    for (key, value) in &top_level {
        if key == "metadata" {
            paths.extend(unknown_notes_paths(value, include_price_fields));
            continue;
        }
        if !is_known_top_level(key, include_price_fields) {
            paths.push(key.clone());
        }
    }
    paths
}

fn is_known_top_level(key: &str, include_price_fields: bool) -> bool {
    FLAT_WIRE_NAME_GROUPS
        .iter()
        .any(|group| group.contains(&key))
        || (include_price_fields
            && (PRICE_COMPOUND_NAMES.contains(&key)
                || PRICE_FLAT_WIRE_NAME_GROUPS
                    .iter()
                    .any(|group| group.contains(&key))))
}

fn unknown_notes_paths(value: &RawValue, include_price_fields: bool) -> Vec<String> {
    let Ok(notes) = serde_json::from_str::<BTreeMap<String, Box<RawValue>>>(value.get()) else {
        return Vec::new();
    };
    notes
        .keys()
        .filter(|key| {
            !(CatalogNotes::WIRE_NAMES.contains(&key.as_str())
                || (include_price_fields && PRICE_METADATA_NAMES.contains(&key.as_str())))
        })
        .map(|key| format!("metadata.{key}"))
        .collect()
}
