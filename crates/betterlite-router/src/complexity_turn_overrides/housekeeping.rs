use crate::complexity_scoring::tier::Tier;
use crate::tier_definitions::checks::ClassifierKind;
use crate::tier_definitions::model::TierSet;

use super::error::TurnOverrideError;
use super::settings::{Settings, active_tier};

// the client-owned title asks LiteLLM routes to the cheapest tier without classifying them
pub const HOUSEKEEPING_ASK_SENTINELS: [&str; 2] = [
    "Write the title in the predominant language of the session",
    "You are coming up with a succinct title for a coding session",
];

// _matched_housekeeping_sentinel: a sentinel on the newest ask alone, never the whole history
#[must_use]
pub fn matched_housekeeping_sentinel(
    settings: &Settings,
    newest_ask: Option<&str>,
) -> Option<String> {
    if settings.classifier_type == ClassifierKind::Custom
        || !settings.route_housekeeping_to_cheapest_tier
    {
        return None;
    }
    let ask = newest_ask.filter(|ask| !ask.is_empty())?;
    if let Some(sentinel) = HOUSEKEEPING_ASK_SENTINELS
        .iter()
        .find(|sentinel| ask.contains(**sentinel))
    {
        return Some((*sentinel).to_owned());
    }
    settings
        .housekeeping_patterns
        .iter()
        .find(|pattern| ask.contains(pattern.as_str()))
        .cloned()
}

// _cheapest_configured_tier: the least severe tier that has models, or None when none does
pub fn cheapest_configured_tier(catalog: &TierSet) -> Result<Option<Tier>, TurnOverrideError> {
    for name in catalog.tier_names() {
        if catalog.pools.get(name).is_some_and(|pool| !pool.is_empty()) {
            return active_tier(catalog, name).map(Some);
        }
    }
    Ok(None)
}
