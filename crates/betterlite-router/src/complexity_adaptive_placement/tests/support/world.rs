use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::sections::complexity_placement as tier_section;
use betterlite_config::sections::context_placement as context_section;
use betterlite_openai::chat::ChatMessage;

use crate::affinity_claims::AffinityClaims;
use crate::affinity_claims::local::LocalPins;
use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::complexity_placement::TierPlacement;
use crate::context_placement::ContextPlacement;
use crate::context_placement::error::PlacementError;
use crate::context_placement::facts::{ModelLimitKey, RouterModelInfo};
use crate::context_placement::request::RequestCounter;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::routing_value_store::StoreSettings;
use crate::tier_definitions::model::TierSet;

use super::tier_set;

pub(super) struct NoLimits;

impl RouterModelInfo for NoLimits {
    fn limit(
        &self,
        _group: &str,
        _deployment: &DeploymentSpec,
        _key: ModelLimitKey,
    ) -> Option<u64> {
        None
    }
}

pub(super) struct NoCounter;

#[async_trait]
impl RequestCounter for NoCounter {
    async fn count(&self, _messages: &[ChatMessage], _text: &str) -> Result<u64, PlacementError> {
        Ok(0)
    }
}

// the collaborators TierPlacement carries, which the tier fallback never reads
pub(crate) struct World {
    pub(super) tiers: TierSet,
    config: tier_section::Config,
    context_config: context_section::Config,
    deployments: DeploymentCatalog,
    claims: AffinityClaims,
    limits: NoLimits,
    counter: NoCounter,
    default_model: Option<String>,
}

impl World {
    pub(crate) fn new(pools: &[(&str, &[&str])], default_model: Option<&str>) -> Self {
        let local = LocalPins::new(
            StoreSettings {
                max_size_in_memory: None,
                default_ttl_seconds: None,
                max_size_per_item: Some(1024_i64),
                loaded_max_size_per_item_kib: 1024_i64,
            },
            Arc::new(|| 0.0_f64),
        )
        .expect("the test pin store starts");
        Self {
            tiers: tier_set(pools),
            config: tier_section::Config {
                return_raw_model_name: false,
                max_tokens_from_tier_model: true,
            },
            context_config: context_section::Config {
                enabled: false,
                buffer: context_section::DEFAULT_BUFFER,
            },
            deployments: DeploymentCatalog::default(),
            claims: AffinityClaims {
                local,
                shared: None,
            },
            limits: NoLimits,
            counter: NoCounter,
            default_model: default_model.map(str::to_owned),
        }
    }

    pub(crate) fn context(&self) -> ContextPlacement<'_> {
        ContextPlacement {
            config: &self.context_config,
            deployments: &self.deployments,
            model_info: &self.limits,
            counter: &self.counter,
        }
    }

    pub(crate) fn placement<'deps>(
        &'deps self,
        context: &'deps ContextPlacement<'deps>,
    ) -> TierPlacement<'deps> {
        TierPlacement {
            config: &self.config,
            tiers: &self.tiers,
            default_model: self.default_model.as_deref(),
            context,
            claims: &self.claims,
            model_name: "probe",
            tier_affinity_config: "probe-affinity",
            deployment_pin: false,
            session_affinity_ttl_seconds: 0,
        }
    }
}
