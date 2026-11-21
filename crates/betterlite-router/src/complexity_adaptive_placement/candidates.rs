use std::collections::BTreeSet;

use betterlite_config::sections::complexity_adaptive_placement::AdaptiveEligible;

use crate::complexity_scoring::tier::Tier;

use super::AdaptivePlacement;
use super::error::AdaptivePlacementError;

// _adaptive_candidate_models: the eligible pool, the context-fit filter, then the plan's bounds
pub struct CandidateQuery<'request> {
    pub classified_tier: &'request Tier,
    pub hard_floor: Option<&'request Tier>,
    pub hard_ceiling: Option<&'request Tier>,
    pub fit_filter: Option<&'request BTreeSet<String>>,
}

impl AdaptivePlacement {
    pub fn candidate_models(
        &self,
        query: &CandidateQuery<'_>,
    ) -> Result<Vec<String>, AdaptivePlacementError> {
        let floor = query
            .hard_floor
            .map(|tier| self.severity_name(tier.name()))
            .transpose()?;
        let ceiling = query
            .hard_ceiling
            .map(|tier| self.severity_name(tier.name()))
            .transpose()?;
        let base = match self.settings.eligible {
            AdaptiveEligible::ClassifiedTier => self
                .pools
                .get(query.classified_tier.name())
                .cloned()
                .unwrap_or_default(),
            AdaptiveEligible::All => self.available_models.clone(),
        };
        let mut candidates = Vec::with_capacity(base.len());
        for model in base {
            if query.fit_filter.is_some_and(|fit| !fit.contains(&model)) {
                continue;
            }
            let severities = self.severities(&model, query.classified_tier)?;
            if floor.is_some_and(|bound| !severities.iter().any(|level| *level >= bound)) {
                continue;
            }
            if ceiling.is_some_and(|bound| !severities.iter().any(|level| *level <= bound)) {
                continue;
            }
            candidates.push(model);
        }
        Ok(candidates)
    }

    // a model is eligible if at least one of its pools satisfies a bound
    pub(super) fn severities(
        &self,
        model: &str,
        classified_tier: &Tier,
    ) -> Result<Vec<u32>, AdaptivePlacementError> {
        let Some(state) = self.state.as_ref() else {
            return Ok(vec![self.severity_name(classified_tier.name())?]);
        };
        let Some(tiers) = state.model_tiers.get(model) else {
            return Ok(vec![self.severity_name(classified_tier.name())?]);
        };
        tiers.iter().map(|tier| self.severity_name(tier)).collect()
    }

    pub(super) fn distance(
        &self,
        model: &str,
        classified_tier: &Tier,
        classified_severity: u32,
    ) -> Result<u32, AdaptivePlacementError> {
        if self.settings.eligible == AdaptiveEligible::ClassifiedTier {
            return Ok(0);
        }
        let severities = self.severities(model, classified_tier)?;
        Ok(severities
            .iter()
            .map(|level| level.abs_diff(classified_severity))
            .min()
            .unwrap_or(0))
    }

    pub(super) fn severity_name(&self, name: &str) -> Result<u32, AdaptivePlacementError> {
        let position = self.tier_names.iter().position(|tier| tier == name);
        position
            .and_then(|position| u32::try_from(position).ok())
            .ok_or_else(|| AdaptivePlacementError::UnknownTier {
                tier: name.to_owned(),
            })
    }
}
