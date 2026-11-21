use betterlite_models::baseline_history::model::BaselineObservation;
use betterlite_models::savings_evidence::snapshot::BaselineCostSnapshot;
use serde::Deserialize;

use crate::autorouter_turn_sql::transaction::AutoRouterTurnTransaction;
use crate::daily_spend_sql::entities::DailySpendEntity;

use super::{DailyBaselineAttribution, DailyBaselineTarget};

// Every key the source record forbids an extra for; turn and daily are required but nullable.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawRecord {
    pub(super) scope: String,
    pub(super) api_key: String,
    pub(super) session_id: String,
    pub(super) router_name: String,
    pub(super) baseline_model: String,
    pub(super) observation: BaselineObservation,
    pub(super) pricing: BaselineCostSnapshot,
    pub(super) turn: Option<AutoRouterTurnTransaction>,
    pub(super) daily: Option<RawDailyAttribution>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawDailyAttribution {
    date: String,
    api_key: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    custom_llm_provider: Option<String>,
    #[serde(default)]
    model_group: Option<String>,
    #[serde(default)]
    endpoint: Option<String>,
    #[serde(default)]
    mcp_namespaced_tool_name: Option<String>,
    #[serde(default)]
    targets: Vec<RawTarget>,
}

impl RawDailyAttribution {
    pub(super) fn into_attribution(self) -> DailyBaselineAttribution {
        DailyBaselineAttribution {
            date: self.date,
            api_key: self.api_key,
            model: self.model,
            custom_llm_provider: self.custom_llm_provider,
            model_group: self.model_group,
            endpoint: self.endpoint,
            mcp_namespaced_tool_name: self.mcp_namespaced_tool_name,
            targets: self
                .targets
                .into_iter()
                .map(RawTarget::into_target)
                .collect(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTarget {
    entity: DailySpendEntity,
    entity_id: Option<String>,
}

impl RawTarget {
    fn into_target(self) -> DailyBaselineTarget {
        DailyBaselineTarget {
            entity: self.entity,
            entity_id: self.entity_id,
        }
    }
}
