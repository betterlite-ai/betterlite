use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use serde::Deserialize;

use betterlite_models::savings_evidence::spend::SavingsSpend;
use betterlite_models::savings_pricing::context::SavingsPricingContext;
use betterlite_models::tiered_pricing::Scalar;

use crate::daily_spend_sql::entities::{
    DailySpendEntity, DailySpendEntityPayload, daily_spend_entity_ids,
};
use crate::daily_spend_sql::records::BaseDailySpendTransaction;
use crate::daily_spend_sql::tags::TagsPayload;

use super::DailyRecordError;
use super::metadata::{RequestStatus, decode_daily_metadata};

mod evidence;
pub mod rows;
mod tests;

// payload.get("metadata", {}), the default get_request_status already reads an absent metadata as.
const DEFAULT_METADATA: &str = "{}";

// SpendLogsPayload of _types.py as it reaches the daily queues: the source reads a partial dict, so presence is kept apart from truthiness.
#[derive(Clone, Debug, Deserialize)]
pub struct DailyPayload {
    #[serde(default, with = "serde_with::rust::double_option")]
    pub user: Option<Option<String>>,
    pub team_id: Option<String>,
    pub organization_id: Option<String>,
    pub end_user: Option<String>,
    pub end_user_id: Option<String>,
    pub agent_id: Option<String>,
    pub request_tags: Option<TagsPayload>,
    #[serde(
        rename = "startTime",
        default,
        with = "serde_with::rust::double_option"
    )]
    pub start_time: Option<Option<Scalar>>,
    #[serde(rename = "endTime")]
    pub end_time: Option<Scalar>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub api_key: Option<Option<String>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub model: Option<Option<String>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub model_group: Option<Option<String>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub custom_llm_provider: Option<Option<String>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub mcp_namespaced_tool_name: Option<Option<String>>,
    pub call_type: Option<String>,
    pub model_id: Option<String>,
    pub request_id: Option<String>,
    pub request_duration_ms: Option<Scalar>,
    pub metadata: Option<String>,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub spend: Option<Decimal>,
}

// The pricing half of the writer's state: B1694's context, the calendar the off-peak rates are read in, and the moment a missing bill time falls back to.
pub struct DailyPricing<'pricing, C> {
    pub context: &'pricing C,
    pub zone: &'pricing TimeZoneDatabase,
    pub fallback_moment: DateTime<Utc>,
}

// B004's call-type map, injected so this module neither owns nor widens ROUTE_ENDPOINT_MAPPING.
pub trait CallTypeEndpoints: Send + Sync {
    fn endpoint(&self, call_type: &str) -> Option<&str>;
}

#[derive(Clone)]
pub struct DailyProjection {
    pub base: BaseDailySpendTransaction,
    pub identities: Vec<Option<String>>,
}

pub(super) struct DailyEvidence {
    pub(super) status: RequestStatus,
    pub(super) internal: bool,
    pub(super) endpoint: Option<String>,
    pub(super) cache_read: i64,
    pub(super) cache_creation: i64,
    pub(super) compression_saved: i64,
    pub(super) savings: SavingsSpend,
    pub(super) timed: Option<i64>,
}

impl DailyPayload {
    pub(super) fn identity(&self) -> DailySpendEntityPayload {
        DailySpendEntityPayload {
            user: self.user.clone(),
            team_id: self.team_id.clone(),
            organization_id: self.organization_id.clone(),
            end_user: self.end_user.clone(),
            agent_id: self.agent_id.clone(),
            request_tags: self.request_tags.clone(),
        }
    }
}

// A present null reads as absent: the source passes payload.get(key) into a field that is None either way.
pub(super) const fn presence(value: Option<&Option<String>>) -> Option<&str> {
    match value {
        Some(Some(text)) => Some(text.as_str()),
        Some(None) | None => None,
    }
}

// _common_add_spend_log_transaction_to_daily_transaction: the entity identities gate the row before the payload's own keys are read.
pub async fn common_daily_transaction<C>(
    payload: &DailyPayload,
    entity: DailySpendEntity,
    pricing: &DailyPricing<'_, C>,
    endpoints: &dyn CallTypeEndpoints,
) -> Result<Option<DailyProjection>, DailyRecordError>
where
    C: SavingsPricingContext,
{
    let identities = common_identities(payload, entity)?;
    if identities.is_empty() {
        return Ok(None);
    }
    if !has_required_keys(payload) {
        return Ok(None);
    }
    let metadata = decode_daily_metadata(
        payload
            .metadata
            .as_deref()
            .unwrap_or(DEFAULT_METADATA)
            .as_bytes(),
    )?;
    let Some(date) = start_date(payload) else {
        return Ok(None);
    };
    let evidence = evidence::gather(payload, &metadata, pricing, endpoints).await?;
    let base = rows::base_transaction(payload, date, &evidence)?;
    Ok(Some(DailyProjection { base, identities }))
}

// Only the end-user entity reads the id the writer projected from end_user; every other identity is the payload's own.
fn common_identities(
    payload: &DailyPayload,
    entity: DailySpendEntity,
) -> Result<Vec<Option<String>>, DailyRecordError> {
    let mut identity = payload.identity();
    if entity == DailySpendEntity::EndUser {
        identity.end_user.clone_from(&payload.end_user_id);
    }
    Ok(daily_spend_entity_ids(&identity, entity)?)
}

// expected_keys and any_expected_keys: an mcp key counts whether or not it holds a value, and its presence alone excuses a missing provider or group; a model names a row only with both of them.
const fn has_required_keys(payload: &DailyPayload) -> bool {
    if payload.start_time.is_none() || payload.api_key.is_none() {
        return false;
    }
    if payload.model.is_none() && payload.mcp_namespaced_tool_name.is_none() {
        return false;
    }
    if payload.mcp_namespaced_tool_name.is_some() {
        return true;
    }
    payload.custom_llm_provider.is_some() && payload.model_group.is_some()
}

// startTime.split("T")[0]: the date is the text before the first T, with no timezone conversion and no datetime path.
fn start_date(payload: &DailyPayload) -> Option<String> {
    let Scalar::Text(text) = payload.start_time.as_ref()?.as_ref()? else {
        return None;
    };
    Some(
        text.split_once('T')
            .map_or_else(|| text.clone(), |(date, _)| date.to_owned()),
    )
}
