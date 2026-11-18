use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::context_placement as section;

use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::complexity_math::window_holds;
use crate::deployment_catalog::scope::all_deployments;
use crate::tier_definitions::model::TierSet;

pub mod error;
pub mod facts;
pub mod fit;
pub mod placement;
pub mod request;
mod tests;

use error::PlacementError;
use facts::{ModelLimitKey, RouterModelInfo, WindowFacts, deployment_limit};
use fit::ContextFit;
use placement::{Placement, PlacementRequest, TokenInput, placement_for_tokens};
use request::{RequestCounter, RequestFacts, out_of_band_text, request_byte_upper_bound};

// the dependencies the context-window gate reads; the catalog and the model-info resolution belong to the router
pub struct ContextPlacement<'deps> {
    pub config: &'deps section::Config,
    pub deployments: &'deps DeploymentCatalog,
    pub model_info: &'deps dyn RouterModelInfo,
    pub counter: &'deps dyn RequestCounter,
}

impl ContextPlacement<'_> {
    // _group_window_facts: the group is only as safe as its smallest member, and a member without a window hides it
    #[must_use]
    pub fn group_window_facts(&self, group: &str) -> WindowFacts {
        let deployments = all_deployments(self.deployments, group, None);
        if deployments.is_empty() {
            return WindowFacts::UNKNOWN;
        }
        let mut windows = Vec::new();
        for deployment in &deployments {
            if let Some(window) = deployment_limit(
                self.model_info,
                group,
                deployment,
                ModelLimitKey::InputTokens,
            ) {
                windows.push(window);
            }
        }
        WindowFacts {
            window: windows.iter().copied().min(),
            has_unknown: windows.len() < deployments.len(),
        }
    }

    // _group_output_ceiling: the smallest output limit, or none when any member declares no limit
    #[must_use]
    pub fn group_output_ceiling(&self, group: &str) -> Option<u64> {
        let deployments = all_deployments(self.deployments, group, None);
        let mut ceilings = Vec::new();
        for deployment in &deployments {
            let ceiling = deployment_limit(
                self.model_info,
                group,
                deployment,
                ModelLimitKey::OutputTokens,
            )?;
            ceilings.push(ceiling);
        }
        ceilings.iter().copied().min()
    }

    // _request_context_fit: count for real only when the byte bound could exceed the smallest known window
    pub async fn request_context_fit(
        &self,
        set: &TierSet,
        default_model: Option<&str>,
        request: &RequestFacts<'_>,
    ) -> ContextFit {
        let buffer = self.config.buffer;
        if request.compaction_pending
            || !self.config.enabled
            || request.messages.is_none_or(|messages| messages.is_empty())
        {
            return ContextFit {
                facts: BTreeMap::new(),
                needed: None,
                buffer,
            };
        }
        let mut names = BTreeSet::new();
        for pool in set.pools.values() {
            for entry in pool.models() {
                names.insert(entry.model_name.as_str());
            }
        }
        if let Some(default) = default_model.filter(|name| !name.is_empty()) {
            names.insert(default);
        }
        let mut facts = BTreeMap::new();
        for name in names {
            facts.insert(name.to_owned(), self.group_window_facts(name));
        }
        let smallest = facts.values().filter_map(|facts| facts.window).min();
        let bound = request_byte_upper_bound(request.messages, &request.out_of_band);
        let needed = match smallest {
            Some(window) if !window_holds(Some(window), Some(bound), buffer) => {
                let text = out_of_band_text(&request.out_of_band);
                match self
                    .counter
                    .count(request.messages.unwrap_or_default(), &text)
                    .await
                {
                    Ok(counted) => Some(counted),
                    Err(error) => {
                        tracing::debug!(%error, "the context-window token count failed");
                        None
                    }
                }
            }
            _ => None,
        };
        ContextFit {
            facts,
            needed,
            buffer,
        }
    }

    // _context_window_placement: only a real count moves a request, and the tier keeps it when part of its pool holds
    pub async fn context_window_placement(
        &self,
        input: &PlacementRequest<'_>,
        context_fit: Option<&ContextFit>,
    ) -> Result<Option<Placement>, PlacementError> {
        if input.request.compaction_pending
            || !self.config.enabled
            || input
                .request
                .messages
                .is_none_or(|messages| messages.is_empty())
        {
            return Ok(None);
        }
        let pool = match input.pool_override {
            Some(overridden) => overridden.to_vec(),
            None => match input.set.pools.get(input.tier.name()) {
                Some(pool) => pool
                    .models()
                    .iter()
                    .map(|entry| entry.model_name.clone())
                    .collect(),
                None => Vec::new(),
            },
        };
        if pool.is_empty() {
            return Ok(None);
        }
        let computed;
        let fit = match context_fit {
            Some(fit) => fit,
            None => {
                computed = self
                    .request_context_fit(input.set, input.default_model, &input.request)
                    .await;
                &computed
            }
        };
        let Some(needed) = fit.needed else {
            return Ok(None);
        };
        let tokens = TokenInput {
            set: input.set,
            tier: input.tier,
            pool: &pool,
            facts: &fit.facts,
            needed,
        };
        placement_for_tokens(self.config, &tokens)
    }
}
