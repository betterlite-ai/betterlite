use betterlite_models::baseline_history::model::BaselineObservation;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::savings_evidence::error::SavingsEvidenceError;
use betterlite_models::savings_evidence::numeric::RoutingDecision;
use betterlite_models::savings_evidence::snapshot::CostBreakdown;
use betterlite_models::savings_pricing::autorouter::AutoRouterDecision;
use betterlite_models::savings_pricing::compare::{SnapshotInput, baseline_cost_snapshot};
use betterlite_models::savings_pricing::context::SavingsPricingContext;
use betterlite_models::token_rates::ModelInfo;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::autorouter_facts::counts_as_present;
use crate::autorouter_turn_sql::transaction::AutoRouterTurnTransaction;
use crate::baseline::flush::queue::{BaselineQueue, QueueError};
use crate::baseline::records::canonical::serialize_record;
use crate::baseline::records::{
    BaselineAccountingRecord, BaselineEntity, DailyBaselineAttribution, DailyBaselineTarget,
    RecordDecodeError, decode_baseline_record,
};
use crate::daily_records::DailyRecordError;
use crate::daily_records::baseline_targets::{
    BaselineTargetEntity, DailyBaselineAttribution as TargetAttribution,
    DailyBaselineTarget as Target, baseline_daily_attribution,
};
use crate::daily_records::projection::DailyPayload;
use crate::redis_spend::codec::MAX_SPEND_BUFFER_BYTES;

use super::{AutorouterQueues, TurnEnvironment};

// The fixed cap the source puts on its baseline list, independent of the queue's own capacity.
pub const BASELINE_QUEUE_LIMIT: usize = 10_000;

// CapturedBaselineObservation of autorouter_baseline_cache.py: the routing hook's frozen record, taken here so spend writing never depends on the hook.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapturedBaselineObservation {
    pub scope: String,
    pub api_key: String,
    pub session_id: String,
    pub router_name: String,
    pub baseline_model: String,
    pub model: String,
    pub prices: Option<ModelInfo>,
    pub observation: BaselineObservation,
}

#[derive(Debug, thiserror::Error)]
pub enum CapturedBaselineError {
    #[error("the captured baseline observation exceeds {MAX_SPEND_BUFFER_BYTES} bytes")]
    TooLarge,
    #[error("the captured baseline observation is not valid JSON of the required shape")]
    Json(#[source] serde_json::Error),
}

// model_validate_json: a recorded capture over the metadata's own text, with no field beyond the eight it declares.
pub fn decode_captured_baseline(
    bytes: &[u8],
) -> Result<CapturedBaselineObservation, CapturedBaselineError> {
    if bytes.len() > MAX_SPEND_BUFFER_BYTES {
        return Err(CapturedBaselineError::TooLarge);
    }
    serde_json::from_slice(bytes).map_err(CapturedBaselineError::Json)
}

// The spend log fields and decoded metadata _enqueue_baseline_accounting reads besides the queues.
pub(crate) struct BaselineCaptureInput<'input> {
    pub(crate) serialized: Option<&'input str>,
    pub(crate) api_key: Option<&'input str>,
    pub(crate) session_id: Option<&'input str>,
    pub(crate) request_id: Option<&'input str>,
    pub(crate) spend: Option<Decimal>,
    pub(crate) turn: Option<&'input AutoRouterTurnTransaction>,
    pub(crate) metadata: &'input JsonObject,
}

// Whether the baseline record took this log's turn, or the caller still has to append it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CaptureOutcome {
    Captured,
    Retained,
}

#[derive(Debug, thiserror::Error)]
pub enum BaselineCaptureError {
    #[error(transparent)]
    Captured(#[from] CapturedBaselineError),
    #[error("the spend log has no api key or session id to match the captured observation against")]
    MissingLogIdentity,
    #[error("the spend log holds no spend for the baseline cost snapshot")]
    MissingSpend,
    #[error("the spend log holds no request id for the captured observation")]
    MissingRequestId,
    #[error("the spend log metadata field {field} is not a JSON object")]
    Mapping { field: &'static str },
    #[error("the baseline cost snapshot could not be built")]
    Snapshot(#[from] SavingsEvidenceError),
    #[error("the baseline daily attribution could not be built")]
    Daily(#[from] DailyRecordError),
    #[error("the baseline accounting record is not one its queue can publish")]
    Record(#[source] RecordDecodeError),
    #[error("the baseline observation queue no longer admits records")]
    Queue(#[source] QueueError),
}

// _enqueue_baseline_accounting: a capture that names this log replaces its turn, and every other answer leaves the turn to the caller.
pub(crate) async fn enqueue_baseline_accounting<C>(
    queues: &AutorouterQueues<'_>,
    environment: &TurnEnvironment<'_, C>,
    daily: &DailyPayload,
    input: &BaselineCaptureInput<'_>,
) -> Result<CaptureOutcome, BaselineCaptureError>
where
    C: SavingsPricingContext,
{
    let Some(serialized) = input.serialized else {
        return Ok(CaptureOutcome::Retained);
    };
    let captured = decode_captured_baseline(serialized.as_bytes())?;
    let (Some(api_key), Some(session_id)) = (input.api_key, input.session_id) else {
        return Err(BaselineCaptureError::MissingLogIdentity);
    };
    if api_key != captured.api_key || session_id != captured.session_id {
        return Ok(CaptureOutcome::Retained);
    }
    let record = accounting_record(captured, environment, daily, input).await?;
    publish(queues, record).await
}

async fn accounting_record<C>(
    captured: CapturedBaselineObservation,
    environment: &TurnEnvironment<'_, C>,
    daily: &DailyPayload,
    input: &BaselineCaptureInput<'_>,
) -> Result<BaselineAccountingRecord, BaselineCaptureError>
where
    C: SavingsPricingContext,
{
    let decision: Option<AutoRouterDecision> =
        typed_mapping(capture_mapping(input.metadata, "routing_decision")?);
    let breakdown: Option<CostBreakdown> =
        typed_mapping(capture_mapping(input.metadata, "cost_breakdown")?);
    let routing_decision = RoutingDecision {
        classifier_cost: decision.and_then(|decision| decision.classifier_cost),
    };
    let pricing = baseline_cost_snapshot(&SnapshotInput {
        model: &captured.model,
        prices: captured.prices.as_ref(),
        actual_spend: input.spend.ok_or(BaselineCaptureError::MissingSpend)?,
        cost_breakdown: breakdown.as_ref(),
        routing_decision: Some(&routing_decision),
    })?;
    let attributed = baseline_daily_attribution(daily, environment.pricing, environment.endpoints)
        .await?
        .map(attribution_record);
    let request_id = input
        .request_id
        .ok_or(BaselineCaptureError::MissingRequestId)?;
    Ok(BaselineAccountingRecord {
        scope: captured.scope,
        api_key: captured.api_key,
        session_id: captured.session_id,
        router_name: captured.router_name,
        baseline_model: captured.baseline_model,
        observation: BaselineObservation {
            request_id: request_id.to_owned(),
            ..captured.observation
        },
        pricing,
        turn: input.turn.cloned(),
        daily: attributed,
    })
}

// `metadata.get(field) or {}` through the mapping adapter: a falsey value reads as the empty mapping, and a truthy non-object fails the record the way validation does.
fn capture_mapping<'meta>(
    metadata: &'meta JsonObject,
    field: &'static str,
) -> Result<Option<&'meta JsonObject>, BaselineCaptureError> {
    let Some(present) = metadata.get(field).filter(|value| counts_as_present(value)) else {
        return Ok(None);
    };
    let Json::Object(mapping) = present else {
        return Err(BaselineCaptureError::Mapping { field });
    };
    Ok(Some(mapping))
}

// The typed keys of a metadata mapping whose other keys the accounting never reads.
fn typed_mapping<T>(mapping: Option<&JsonObject>) -> Option<T>
where
    T: DeserializeOwned,
{
    let mapping = mapping?;
    let text = serde_json::to_string(mapping).ok()?;
    serde_json::from_str(&text).ok()
}

async fn publish(
    queues: &AutorouterQueues<'_>,
    record: BaselineAccountingRecord,
) -> Result<CaptureOutcome, BaselineCaptureError> {
    // The record is a strict model: its own canonical decoder is the validator the queue will apply.
    let bytes = serialize_record(&record).into_bytes();
    let record = decode_baseline_record(&bytes).map_err(BaselineCaptureError::Record)?;
    if baseline_pending(queues.baseline) >= BASELINE_QUEUE_LIMIT {
        tracing::warn!("Auto-router baseline observation queue is full");
        return Ok(CaptureOutcome::Retained);
    }
    queues
        .baseline
        .push(record)
        .await
        .map_err(BaselineCaptureError::Queue)?;
    queues.logs.request_flush();
    Ok(CaptureOutcome::Captured)
}

// The queue exposes no length; a lease takes every pending record and its Drop puts them back.
pub(crate) fn baseline_pending(queue: &BaselineQueue) -> usize {
    let lease = queue.lease(usize::MAX);
    let pending = lease.records.len();
    drop(lease);
    pending
}

fn attribution_record(attribution: TargetAttribution) -> DailyBaselineAttribution {
    DailyBaselineAttribution {
        date: attribution.date,
        api_key: attribution.api_key,
        model: attribution.model,
        custom_llm_provider: attribution.custom_llm_provider,
        model_group: attribution.model_group,
        endpoint: attribution.endpoint,
        mcp_namespaced_tool_name: attribution.mcp_namespaced_tool_name,
        targets: attribution
            .targets
            .into_iter()
            .map(attribution_target)
            .collect(),
    }
}

fn attribution_target(target: Target) -> DailyBaselineTarget {
    DailyBaselineTarget {
        entity: record_entity(target.entity),
        entity_id: target.entity_id,
    }
}

const fn record_entity(entity: BaselineTargetEntity) -> BaselineEntity {
    match entity {
        BaselineTargetEntity::User => BaselineEntity::User,
        BaselineTargetEntity::Team => BaselineEntity::Team,
        BaselineTargetEntity::Org => BaselineEntity::Org,
        BaselineTargetEntity::EndUser => BaselineEntity::EndUser,
        BaselineTargetEntity::Agent => BaselineEntity::Agent,
        BaselineTargetEntity::Tag => BaselineEntity::Tag,
    }
}
