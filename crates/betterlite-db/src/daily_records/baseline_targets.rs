use betterlite_models::savings_pricing::context::SavingsPricingContext;
use serde::Serialize;

use crate::daily_spend_sql::entities::{DailySpendEntity, daily_spend_entity_ids};

use super::DailyRecordError;
use super::projection::{
    CallTypeEndpoints, DailyPayload, DailyPricing, DailyProjection, common_daily_transaction,
};

mod tests;

// DailySpendEntity of payouts' attribution, in DAILY_SPEND_TABLES order so the first available base is the source's.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BaselineTargetEntity {
    User,
    Team,
    Org,
    EndUser,
    Agent,
    Tag,
}

const TABLE_ORDER: [BaselineTargetEntity; 6] = [
    BaselineTargetEntity::User,
    BaselineTargetEntity::Team,
    BaselineTargetEntity::Org,
    BaselineTargetEntity::EndUser,
    BaselineTargetEntity::Agent,
    BaselineTargetEntity::Tag,
];

impl BaselineTargetEntity {
    const fn spend_entity(self) -> DailySpendEntity {
        match self {
            Self::User => DailySpendEntity::User,
            Self::Team => DailySpendEntity::Team,
            Self::Org => DailySpendEntity::Org,
            Self::EndUser => DailySpendEntity::EndUser,
            Self::Agent => DailySpendEntity::Agent,
            Self::Tag => DailySpendEntity::Tag,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DailyBaselineTarget {
    pub entity: BaselineTargetEntity,
    pub entity_id: Option<String>,
}

// DailyBaselineAttribution of baseline_accounting.py: the base row the savings are compared against, and every entity it can be published to.
#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct DailyBaselineAttribution {
    pub date: String,
    pub api_key: String,
    pub model: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub model_group: Option<String>,
    pub endpoint: Option<String>,
    pub mcp_namespaced_tool_name: Option<String>,
    pub targets: Vec<DailyBaselineTarget>,
}

// _baseline_daily_attribution: every daily base is built from the end-user-normalized payload, the first that exists names the row, and every target it can reach is listed.
pub async fn baseline_daily_attribution<C>(
    payload: &DailyPayload,
    pricing: &DailyPricing<'_, C>,
    endpoints: &dyn CallTypeEndpoints,
) -> Result<Option<DailyBaselineAttribution>, DailyRecordError>
where
    C: SavingsPricingContext,
{
    let mut normalized = payload.clone();
    normalized.end_user_id.clone_from(&payload.end_user);
    let mut projections = Vec::with_capacity(TABLE_ORDER.len());
    for entity in TABLE_ORDER {
        let projection =
            common_daily_transaction(&normalized, entity.spend_entity(), pricing, endpoints)
                .await?;
        projections.push((entity, projection));
    }
    let Some(base) = projections
        .iter()
        .find_map(|(_, projection)| projection.as_ref())
    else {
        return Ok(None);
    };
    let targets = targets_from(payload, &projections)?;
    Ok(Some(DailyBaselineAttribution {
        date: base.base.date.clone(),
        api_key: base.base.api_key.clone(),
        model: base.base.model.clone(),
        custom_llm_provider: base.base.custom_llm_provider.clone(),
        model_group: base.base.model_group.clone(),
        endpoint: base.base.endpoint.clone(),
        mcp_namespaced_tool_name: base.base.mcp_namespaced_tool_name.clone(),
        targets,
    }))
}

fn targets_from(
    payload: &DailyPayload,
    projections: &[(BaselineTargetEntity, Option<DailyProjection>)],
) -> Result<Vec<DailyBaselineTarget>, DailyRecordError> {
    let identity = payload.identity();
    let mut targets = Vec::new();
    for (entity, projection) in projections {
        if projection.is_none() {
            continue;
        }
        for entity_id in daily_spend_entity_ids(&identity, entity.spend_entity())? {
            targets.push(DailyBaselineTarget {
                entity: *entity,
                entity_id,
            });
        }
    }
    Ok(targets)
}
