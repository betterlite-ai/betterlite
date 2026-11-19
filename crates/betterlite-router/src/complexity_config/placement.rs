use std::collections::BTreeSet;

use crate::complexity_config::error::ComplexityConfigError;
use crate::complexity_config::keys::COMPLEXITY_ROUTER_CONFIG_KEYS;
use crate::tier_definitions::model::TierSet;

// a router setting written into a tier entry's request params configures nothing and reaches the provider
pub fn validate(set: &TierSet) -> Result<(), ComplexityConfigError> {
    let placements = misplaced(set);
    if placements.is_empty() {
        return Ok(());
    }
    Err(ComplexityConfigError::MisplacedTierParams { placements })
}

fn misplaced(set: &TierSet) -> Vec<String> {
    let mut placements = Vec::new();
    for (tier, pool) in &set.pools {
        for entry in pool.models() {
            let keys: BTreeSet<&str> = entry
                .params
                .keys()
                .map(String::as_str)
                .filter(|key| COMPLEXITY_ROUTER_CONFIG_KEYS.contains(key))
                .collect();
            placements.extend(keys.into_iter().map(|key| format!("{tier}.{key}")));
        }
    }
    placements
}
