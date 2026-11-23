use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::savings_evidence::billed_at::BilledAt;
use betterlite_models::savings_evidence::injection::marks_gateway_injection;
use betterlite_models::savings_evidence::metadata::Metadata;
use betterlite_models::savings_evidence::snapshot::CostBreakdown;
use betterlite_models::savings_evidence::usage::RawUsageObject;
use betterlite_models::savings_pricing::autorouter::AutoRouterDecision;
use betterlite_models::savings_pricing::context::SavingsPricingContext;
use betterlite_models::savings_pricing::spend::{SavingsSpendInput, compute_savings_spend};
use betterlite_models::savings_pricing::usage::request_usage;
use betterlite_models::tiered_pricing::Scalar;
use rust_decimal::Decimal;
use serde::de::DeserializeOwned;

use crate::autorouter_facts::AutorouterFactsError;
use crate::autorouter_facts::turn::turn_transaction;
use crate::autorouter_turn_sql::transaction::AutoRouterTurnTransaction;
use crate::daily_records::projection::{DailyPayload, DailyPricing};

use super::baseline_capture::{BaselineCaptureInput, CaptureOutcome, enqueue_baseline_accounting};
use super::payload::{AutorouterPayload, decode_autorouter_payload};
use super::queues::UsageQueueError;
use super::{AutorouterQueues, TurnEnvironment};

#[derive(Debug, thiserror::Error)]
pub enum TurnEnqueueError {
    #[error("the spend log payload could not be read")]
    Payload(#[source] serde_json::Error),
    #[error("the spend log metadata is not a JSON object")]
    Metadata(#[source] serde_json::Error),
    #[error("the spend log metadata evidence could not be read")]
    Evidence(#[source] serde_json::Error),
    #[error("the spend log end time is not a bill time")]
    EndTime,
    #[error("the auto-router turn could not be built")]
    Turn(#[source] AutorouterFactsError),
    #[error("the auto-router turn queue no longer admits transactions")]
    Queue(#[source] UsageQueueError),
}

// _enqueue_autorouter_turn_transaction: an auto-routed request's savings, its rollup turn, and the captured baseline that replaces that turn when it names this log.
pub async fn enqueue_autorouter_turn<C>(
    queues: Option<&AutorouterQueues<'_>>,
    environment: &TurnEnvironment<'_, C>,
    payload: &[u8],
) where
    C: SavingsPricingContext,
{
    let Some(wired) = queues else {
        return;
    };
    if let Err(error) = turn_enqueue(wired, environment, payload).await {
        tracing::debug!("_enqueue_autorouter_turn_transaction error (non-blocking): {error}");
    }
}

pub(crate) async fn turn_enqueue<C>(
    queues: &AutorouterQueues<'_>,
    environment: &TurnEnvironment<'_, C>,
    payload: &[u8],
) -> Result<(), TurnEnqueueError>
where
    C: SavingsPricingContext,
{
    let decoded = decode_autorouter_payload(payload).map_err(TurnEnqueueError::Payload)?;
    let Some(metadata_text) = decoded.metadata.as_deref().filter(|text| !text.is_empty()) else {
        return Ok(());
    };
    let metadata = turn_metadata(metadata_text)?;
    if !routes_turn(&metadata.object) {
        return Ok(());
    }
    let saved_spend = savings_spend(&decoded.daily, &metadata, environment.pricing).await?;
    let built = turn_transaction(&decoded.turn, &metadata.object, saved_spend)
        .map_err(TurnEnqueueError::Turn)?;
    let captured = enqueue_baseline_accounting(
        queues,
        environment,
        &decoded.daily,
        &capture_input(&decoded, &metadata, built.as_ref()),
    )
    .await;
    match captured {
        Ok(CaptureOutcome::Captured) => return Ok(()),
        Ok(CaptureOutcome::Retained) => {}
        Err(error) => {
            tracing::warn!(
                "Auto-router baseline observation was unavailable; actual turn retained: {error}"
            );
        }
    }
    let Some(transaction) = built else {
        return Ok(());
    };
    queues
        .turns
        .push(transaction)
        .await
        .map_err(TurnEnqueueError::Queue)
}

// The metadata mapping the source validates before it reads any turn field, with the savings evidence and typed sub-mappings the enqueue prices.
struct TurnMetadata {
    object: JsonObject,
    savings: Metadata,
    decision: Option<AutoRouterDecision>,
    usage: Option<RawUsageObject>,
    breakdown: Option<CostBreakdown>,
}

fn turn_metadata(text: &str) -> Result<TurnMetadata, TurnEnqueueError> {
    let object: JsonObject = serde_json::from_str(text).map_err(TurnEnqueueError::Metadata)?;
    let savings: Metadata = serde_json::from_str(text).map_err(TurnEnqueueError::Evidence)?;
    Ok(TurnMetadata {
        decision: object_value(object.get("routing_decision")),
        usage: object_value(object.get("usage_object")),
        breakdown: object_value(object.get("cost_breakdown")),
        savings,
        object,
    })
}

// `value if isinstance(value, Mapping) else None`, with an object the typed record cannot read left to its None the way the source's own readers swallow it.
fn object_value<T>(value: Option<&Json>) -> Option<T>
where
    T: DeserializeOwned,
{
    let present = value?;
    let Json::Object(_) = present else {
        return None;
    };
    let text = serde_json::to_string(present).ok()?;
    serde_json::from_str(&text).ok()
}

// `isinstance(routing_decision, Mapping) and routing_decision`: only a mapping holding a key at all names a routing decision.
fn routes_turn(metadata: &JsonObject) -> bool {
    let Some(Json::Object(decision)) = metadata.get("routing_decision") else {
        return false;
    };
    !decision.is_empty()
}

// compute_savings_spend of the turn path: no compression credit, and only a mapping-shaped recorded estimate is authoritative.
async fn savings_spend<C>(
    payload: &DailyPayload,
    metadata: &TurnMetadata,
    pricing: &DailyPricing<'_, C>,
) -> Result<Decimal, TurnEnqueueError>
where
    C: SavingsPricingContext,
{
    let usage = request_usage(metadata.usage.as_ref());
    let billed_at = billed_at(payload)?;
    let spend = compute_savings_spend(
        pricing.context,
        &SavingsSpendInput {
            model: present(payload.model.as_ref()),
            custom_llm_provider: present(payload.custom_llm_provider.as_ref()),
            compression_saved_tokens: 0,
            gateway_injected_cache: marks_gateway_injection(
                Some(&metadata.savings),
                payload.model_id.as_deref(),
            ),
            routing_decision: metadata.decision.as_ref(),
            usage: usage.as_ref(),
            model_id: payload.model_id.as_deref(),
            cost_breakdown: metadata.breakdown.as_ref(),
            recorded_autorouter_savings: metadata.savings.autorouter_savings.as_ref(),
            recorded_estimate: metadata
                .savings
                .autorouter_savings_estimate
                .as_ref()
                .filter(|estimate| estimate.as_mapping().is_some()),
            billed_at: &billed_at,
            fallback_moment: pricing.fallback_moment,
            database: pricing.zone,
        },
    )
    .await;
    Ok(spend.autorouter)
}

// A present null reads as absent: the source passes the value into a field that is None either way.
const fn present(value: Option<&Option<String>>) -> Option<&str> {
    match value {
        Some(Some(text)) => Some(text.as_str()),
        Some(None) | None => None,
    }
}

// payload.get("endTime") through _coerce_billed_at: only logged text is a bill time, and a value it cannot read fails the way its AttributeError does.
pub(crate) fn billed_at(payload: &DailyPayload) -> Result<BilledAt, TurnEnqueueError> {
    match payload.end_time.as_ref() {
        None | Some(Scalar::Null) => Ok(BilledAt::Missing),
        Some(Scalar::Text(text)) => Ok(BilledAt::Text(text.clone())),
        Some(Scalar::Bool(_) | Scalar::Number(_) | Scalar::Array(_) | Scalar::Object(_)) => {
            Err(TurnEnqueueError::EndTime)
        }
    }
}

// The spend log fields and decoded metadata _enqueue_baseline_accounting reads for this log.
fn capture_input<'input>(
    payload: &'input AutorouterPayload,
    metadata: &'input TurnMetadata,
    turn: Option<&'input AutoRouterTurnTransaction>,
) -> BaselineCaptureInput<'input> {
    BaselineCaptureInput {
        serialized: serialized_capture(&metadata.object),
        api_key: payload
            .turn
            .api_key
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        session_id: payload.turn.session_id.as_deref(),
        request_id: payload.daily.request_id.as_deref(),
        spend: payload.daily.spend,
        turn,
        metadata: &metadata.object,
    }
}

// isinstance(metadata.get("autorouter_baseline_observation"), str): only text names a capture, an empty one included.
fn serialized_capture(metadata: &JsonObject) -> Option<&str> {
    match metadata.get("autorouter_baseline_observation") {
        Some(Json::Text(text)) => Some(text),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            None
        }
    }
}
