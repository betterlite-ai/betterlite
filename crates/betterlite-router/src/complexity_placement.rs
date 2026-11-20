use std::collections::BTreeMap;

use betterlite_config::sections::complexity_placement as section;
use rand::Rng;

use crate::affinity_claims::AffinityClaims;
use crate::complexity_scoring::tier::Tier;
use crate::context_placement::ContextPlacement;
use crate::router_predicates::JsonMap;
use crate::tier_definitions::model::TierSet;

pub mod error;
pub mod params;
pub mod params_on_request;
pub mod pin;
pub mod pools;
mod tests;

use self::error::TierPlacementError;

// the model a tier decided on, with the pool it was drawn from and the params the tier pins onto the request
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedTier {
    pub tier: String,
    pub model: String,
    pub candidates: Vec<String>,
    pub params: JsonMap,
}

// the pin a continuation replays: the model held and the tier it was decided for
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetainedPin {
    pub model: String,
    pub tier: Option<String>,
}

// one tier placement request: the tier, a caller-supplied pool override, and the pin a continuation carries
pub struct TierChoice<'request> {
    pub tier: &'request Tier,
    pub allowed_models: Option<&'request [String]>,
    pub retained_pin: Option<&'request RetainedPin>,
}

// a drawn model and the candidates the pin may hold it among
pub struct Selection<'model> {
    pub model: &'model str,
    pub candidates: &'model [String],
}

// _pick_model_for_tier's inputs besides the request: the pools, the default model and the affinity pin store
pub struct TierPlacement<'deps> {
    pub config: &'deps section::Config,
    pub tiers: &'deps TierSet,
    pub default_model: Option<&'deps str>,
    pub context: &'deps ContextPlacement<'deps>,
    pub claims: &'deps AffinityClaims,
    pub model_name: &'deps str,
    pub tier_affinity_config: &'deps str,
    pub deployment_pin: bool,
    pub session_affinity_ttl_seconds: u32,
}

impl TierPlacement<'_> {
    // _tier_pools: every configured tier as its model names, a single value as a one-model list.
    #[must_use]
    pub fn tier_pools(&self) -> BTreeMap<String, Vec<String>> {
        pools::tier_pools(self.tiers)
    }

    // get_model_for_tier: the tier's own pool, then the default model, then MEDIUM.
    pub fn model_for_tier<R>(
        &self,
        tier: &Tier,
        random: &mut R,
    ) -> Result<String, TierPlacementError>
    where
        R: Rng + ?Sized,
    {
        pools::model_for_tier(self, tier, random)
    }

    // _litellm_params_for_model: the tier entry's params, plus the model's output ceiling as max_tokens.
    #[must_use]
    pub fn params_for_model(&self, tier: Option<&Tier>, model: &str) -> JsonMap {
        params::params_for_model(self, tier, model)
    }

    // _pin_model_for_tier: a session's tier pin holds one model over the pool's own draw.
    pub async fn pin_model_for_tier(
        &self,
        tier: &Tier,
        selection: &Selection<'_>,
        request_kwargs: &mut JsonMap,
        retained_pin: Option<&RetainedPin>,
    ) -> Result<String, TierPlacementError> {
        pin::pin_model_for_tier(self, tier, selection, request_kwargs, retained_pin).await
    }

    // _pick_model_for_tier without plugins: draw or keep the pin, then read the placed model's tier params.
    pub async fn place_tier<R>(
        &self,
        choice: &TierChoice<'_>,
        request_kwargs: &mut JsonMap,
        random: &mut R,
    ) -> Result<PlacedTier, TierPlacementError>
    where
        R: Rng + ?Sized,
    {
        let tier_key = choice.tier.name();
        let (selected, candidates) = match choice.allowed_models {
            Some(allowed) => (
                pools::pick_from_names(allowed, tier_key, random)?,
                allowed.to_vec(),
            ),
            None => {
                let candidates = self
                    .tiers
                    .pools
                    .get(tier_key)
                    .map_or_else(Vec::new, pools::model_names);
                (self.model_for_tier(choice.tier, random)?, candidates)
            }
        };
        let selection = Selection {
            model: &selected,
            candidates: &candidates,
        };
        let model = self
            .pin_model_for_tier(choice.tier, &selection, request_kwargs, choice.retained_pin)
            .await?;
        let params = self.params_for_model(Some(choice.tier), &model);
        Ok(PlacedTier {
            tier: tier_key.to_owned(),
            model,
            candidates,
            params,
        })
    }

    // async_pre_routing_hook's raw-model-name marker: keep the resolved model name in the response.
    pub fn mark_raw_model_name(&self, request_kwargs: &mut JsonMap) {
        params_on_request::mark_raw_model_name(self.config, request_kwargs);
    }
}
