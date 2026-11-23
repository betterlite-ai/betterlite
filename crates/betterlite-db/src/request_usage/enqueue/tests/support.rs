use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Arc;
use std::time::Duration;

use betterlite_models::model_registration::JsonObject;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::savings_pricing::context::{ModelIdentity, SavingsPricingContext};
use chrono::DateTime;
use jiff::tz::TimeZoneDatabase;
use tokio::sync::Notify;
use tokio::time::timeout;

use crate::autorouter_turn_sql::transaction::AutoRouterTurnTransaction;
use crate::baseline::flush::queue::BaselineQueue;
use crate::baseline::records::{BaselineAccountingRecord, decode_baseline_record};
use crate::daily_records::projection::{CallTypeEndpoints, DailyPayload, DailyPricing};
use crate::request_usage::baseline_capture::{
    BaselineCaptureError, BaselineCaptureInput, CaptureOutcome, enqueue_baseline_accounting,
};
use crate::request_usage::queues::UsageQueue;
use crate::request_usage::turn_enqueue::{TurnEnqueueError, enqueue_autorouter_turn, turn_enqueue};
use crate::request_usage::{AutorouterQueues, TurnEnvironment};
use crate::spend_log_storage::rows::SpendLogQueue;

// A canonical record B1699 stores: the cap test only needs records the queue admits, not the capture that built them.
const TEMPLATE_RECORD: &str = r#"{"scope":"autorouter-baseline:v3:0000000000000000000000000000000000000000000000000000000000000000","api_key":"sk-capture","session_id":"sess-1","router_name":"auto-router","baseline_model":"anthropic/claude-opus-5","observation":{"request_id":"req-template","started_at":1.5,"available_at":2.0,"outcome":"complete","baseline_equivalent":true},"pricing":{"model":"m","provider":"anthropic","prices":null,"basis":[null,null,null],"actual_spend":0.5,"classifier_cost":0.0},"turn":null,"daily":null}"#;

// The routes and public rates a case injects: one exact model identity and no deployment registry behind it.
pub(super) struct Environment {
    prices: CasePrices,
    zone: TimeZoneDatabase,
    pub(super) endpoints: CaseEndpoints,
}

impl Environment {
    pub(super) fn new() -> Self {
        Self {
            prices: CasePrices {
                prices: BTreeMap::new(),
            },
            zone: TimeZoneDatabase::none(),
            endpoints: CaseEndpoints,
        }
    }

    fn pricing(&self) -> DailyPricing<'_, CasePrices> {
        DailyPricing {
            context: &self.prices,
            zone: &self.zone,
            fallback_moment: DateTime::UNIX_EPOCH,
        }
    }
}

struct CasePrices {
    prices: BTreeMap<String, PriceRecord>,
}

impl CasePrices {
    // The priced form of one stored record, with no tiered or off-peak tables behind it.
    fn priced(&self, key: &str) -> Option<PricedModel> {
        let record = self.prices.get(key)?.clone();
        Some(PricedModel {
            record,
            tiered_pricing: Vec::new(),
            off_peak_pricing: None,
            off_peak_cache_read_present: false,
        })
    }
}

// A rate key is the provider/model spelling, with an empty provider when the caller names none.
impl PriceLookup for CasePrices {
    fn effective_model_info(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        self.raw_cost_map_entry(&format!("{}/{model}", custom_llm_provider.unwrap_or("")))
    }

    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord> {
        self.prices.get(key).cloned()
    }

    fn effective_priced_model(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        self.priced(&format!("{}/{model}", custom_llm_provider.unwrap_or("")))
    }
}

impl SavingsPricingContext for CasePrices {
    fn resolve_model(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<ModelIdentity> {
        let identity = ModelIdentity {
            model: model.to_owned(),
            provider: custom_llm_provider.unwrap_or_default().to_owned(),
        };
        Some(identity)
    }

    fn deployment_price(
        &self,
        _deployment_id: Option<&str>,
        _model: &str,
    ) -> impl Future<Output = Option<PricedModel>> + Send {
        ready(None)
    }

    fn public_lookup(&self) -> &dyn PriceLookup {
        self
    }
}

// No call type maps to an endpoint in these cases, so a route never decides a row.
pub(super) struct CaseEndpoints;

impl CallTypeEndpoints for CaseEndpoints {
    fn endpoint(&self, _call_type: &str) -> Option<&str> {
        None
    }
}

// The queues one turn enqueue writes, with the flush signal the capture must wake.
pub(super) struct TurnCase {
    environment: Environment,
    turns: UsageQueue<AutoRouterTurnTransaction>,
    baseline: BaselineQueue,
    logs: SpendLogQueue,
    signal: Arc<Notify>,
}

impl TurnCase {
    pub(super) fn new() -> Self {
        let logs = SpendLogQueue::new(0x0010_0000);
        let signal = Arc::new(Notify::new());
        logs.set_flush_signal(Some(Arc::clone(&signal)));
        Self {
            environment: Environment::new(),
            turns: UsageQueue::new(64).unwrap(),
            baseline: BaselineQueue::new(20_000).unwrap(),
            logs,
            signal,
        }
    }

    // The public record one rate key answers with, as the provider/model spelling the lookup receives.
    pub(super) fn add_price(&mut self, key: &str, record: &str) {
        self.environment.prices.prices.insert(
            key.to_owned(),
            serde_json::from_str::<PriceRecord>(record).unwrap(),
        );
    }

    pub(super) async fn enqueue(&self, payload: &str) {
        let environment = TurnEnvironment {
            pricing: &self.environment.pricing(),
            endpoints: &self.environment.endpoints,
        };
        enqueue_autorouter_turn(Some(&self.queues()), &environment, payload.as_bytes()).await;
    }

    // The same enqueue with no router catalog in scope, which the source's get_llm_router leaves None.
    pub(super) async fn enqueue_without_queues(&self, payload: &str) {
        let environment = TurnEnvironment {
            pricing: &self.environment.pricing(),
            endpoints: &self.environment.endpoints,
        };
        enqueue_autorouter_turn(None, &environment, payload.as_bytes()).await;
    }

    pub(super) async fn enqueue_result(&self, payload: &str) -> Result<(), TurnEnqueueError> {
        let pricing = self.environment.pricing();
        let environment = TurnEnvironment {
            pricing: &pricing,
            endpoints: &self.environment.endpoints,
        };
        turn_enqueue(&self.queues(), &environment, payload.as_bytes()).await
    }

    pub(super) async fn enqueue_baseline(
        &self,
        daily: &DailyPayload,
        input: &BaselineCaptureInput<'_>,
    ) -> Result<CaptureOutcome, BaselineCaptureError> {
        let pricing = self.environment.pricing();
        let environment = TurnEnvironment {
            pricing: &pricing,
            endpoints: &self.environment.endpoints,
        };
        enqueue_baseline_accounting(&self.queues(), &environment, daily, input).await
    }

    pub(super) async fn fill_baseline(&self, count: usize) {
        let record = template_record();
        for _ in 0..count {
            self.baseline.push(record.clone()).await.unwrap();
        }
    }

    pub(super) fn close_turns(&self) -> usize {
        self.turns.close()
    }

    fn queues(&self) -> AutorouterQueues<'_> {
        AutorouterQueues {
            turns: &self.turns,
            baseline: &self.baseline,
            logs: &self.logs,
        }
    }

    pub(super) fn turns(&self) -> Vec<AutoRouterTurnTransaction> {
        self.turns.drain(usize::MAX)
    }

    pub(super) fn turn_count(&self) -> usize {
        self.turns.len()
    }

    pub(super) fn baseline_records(&self) -> Vec<BaselineAccountingRecord> {
        let lease = self.baseline.lease(usize::MAX);
        let records: Vec<BaselineAccountingRecord> = lease
            .records
            .iter()
            .map(|queued| queued.record.clone())
            .collect();
        drop(lease);
        records
    }

    pub(super) async fn flushed(&self) -> bool {
        timeout(Duration::from_millis(50), self.signal.notified())
            .await
            .is_ok()
    }
}

pub(super) fn bounded<T>(capacity: i64) -> UsageQueue<T> {
    UsageQueue::new(capacity).unwrap()
}

pub(super) fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub(super) fn captured(api_key: &str, session_id: &str) -> String {
    format!(
        r#"{{"scope":"autorouter-baseline:v3:0000000000000000000000000000000000000000000000000000000000000000","api_key":"{api_key}","session_id":"{session_id}","router_name":"auto-router","baseline_model":"anthropic/claude-opus-5","model":"anthropic/claude-sonnet-5","prices":null,"observation":{{"version":3,"request_id":"req-captured","started_at":1.5,"available_at":2.0,"outcome":"complete","baseline_equivalent":true}}}}"#
    )
}

// The metadata a routed request logs, with the capture only when the case records one.
pub(super) fn turn_metadata(capture: Option<&str>) -> String {
    let mut metadata = String::from(
        r#"{"routing_decision":{"router_model_name":"auto-router","savings_baseline_model":"anthropic/claude-opus-5"}"#,
    );
    if let Some(observation) = capture {
        metadata.push_str(r#","autorouter_baseline_observation":"#);
        metadata.push_str(&serde_json::to_string(observation).unwrap());
    }
    metadata.push('}');
    metadata
}

// A successful routed request whose daily attribution and turn both build.
pub(super) fn turn_payload(metadata: &str) -> String {
    let quoted = serde_json::to_string(metadata).unwrap();
    format!(
        r#"{{"status":"success","model":"anthropic/claude-sonnet-5","model_group":"claude-sonnet-5","api_key":"sk-capture","user":"u1","session_id":"sess-1","startTime":"2026-01-02T03:04:05Z","endTime":"2026-01-02T03:04:06Z","prompt_tokens":3,"completion_tokens":4,"spend":0.25,"request_id":"req-log","model_id":"dep-1","custom_llm_provider":"anthropic","metadata":{quoted}}}"#
    )
}

pub(super) fn template_record() -> BaselineAccountingRecord {
    decode_baseline_record(TEMPLATE_RECORD.as_bytes()).unwrap()
}

pub(super) fn metadata_object(text: &str) -> JsonObject {
    serde_json::from_str(text).unwrap()
}

// An empty mapping for the accounting input a direct call needs.
pub(super) fn empty_metadata() -> JsonObject {
    JsonObject::new()
}
