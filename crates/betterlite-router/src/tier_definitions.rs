use betterlite_config::sections::tier_definitions as section;

pub mod checks;
pub mod definitions;
pub mod error;
pub mod labels;
pub mod model;

mod tests;

use self::checks::TierChecks;
use self::error::TierDefinitionsError;
use self::model::TierSet;

impl TierSet {
    // the typed, validated tier configuration one deployment's complexity_router_config describes
    pub fn resolve(
        input: &section::Config,
        checks: &TierChecks<'_>,
    ) -> Result<Self, TierDefinitionsError> {
        let pools = definitions::resolve_pools(input)?;
        let ladder = input
            .tier_definitions
            .as_deref()
            .map(definitions::normalize_definitions)
            .transpose()?;
        let display = labels::resolve_labels(&input.tier_labels)?;
        definitions::validate_non_reasoning(
            input.non_reasoning_tier,
            ladder.is_some(),
            checks.classifier_type,
            &pools,
        )?;
        definitions::validate_definitions(ladder.as_deref(), &pools, &display, checks)?;
        labels::validate_labels(&display, input.non_reasoning_tier)?;
        Ok(Self {
            definitions: ladder,
            pools,
            labels: display,
            non_reasoning: input.non_reasoning_tier,
            fallback: checks.fallback_tier.map(str::to_owned),
        })
    }
}
