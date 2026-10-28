use super::{PriceFieldError, decimal_at, values::PRICE_KEYS};
use serde_json::value::RawValue;
use std::collections::BTreeMap;

pub(super) fn validate(
    object: &BTreeMap<String, Box<RawValue>>,
    prefix: &str,
) -> Result<(), PriceFieldError> {
    for (name, value) in object {
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}.{name}")
        };
        if monetary(prefix, name) {
            decimal_at(&path, value)?;
            continue;
        }
        if prefix.is_empty()
            && matches!(
                name.as_str(),
                "provider_specific_entry"
                    | "guardrail_cost_per_unit"
                    | "metadata"
                    | "sample_spec"
                    | "off_peak_pricing"
                    | "search_context_cost_per_query"
            )
        {
            let nested = serde_json::from_str(value.get())
                .map_err(|source| PriceFieldError::Json { source })?;
            validate(&nested, &path)?;
        }
        if prefix.is_empty() && name == "tiered_pricing" {
            let tiers: Vec<BTreeMap<String, Box<RawValue>>> = serde_json::from_str(value.get())
                .map_err(|source| PriceFieldError::Json { source })?;
            for (index, tier) in tiers.iter().enumerate() {
                validate(tier, &format!("{path}[{index}]"))?;
            }
        }
    }
    Ok(())
}

fn monetary(prefix: &str, name: &str) -> bool {
    match prefix {
        "guardrail_cost_per_unit" | "search_context_cost_per_query" => true,
        "provider_specific_entry" => matches!(name, "us" | "fast"),
        "metadata" => matches!(
            name,
            "original_pricing_per_hour" | "original_pricing_per_minute"
        ),
        _ => PRICE_KEYS.contains(&name),
    }
}
