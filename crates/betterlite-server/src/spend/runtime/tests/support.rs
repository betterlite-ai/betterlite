#![cfg(test)]

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::error::Error as StdError;
use std::future::ready;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use betterlite_db::daily_records::projection::CallTypeEndpoints;
use betterlite_db::queues::pod_lock::NoPodLockEventSink;
use betterlite_db::queues::pod_lock::owner::PodId;
use betterlite_db::redis_spend::events::NoRedisBufferEventSink;
use betterlite_db::spend_flush::service::{ServiceFuture, SpendLogsService};
use betterlite_db::spend_queues::entity::SpendAmount;
use betterlite_db::spend_queues::events::NoSpendQueueEventSink;
use betterlite_db::spend_writer::request::{PayloadFuture, SpendPayloadSource, SpendUpdateRequest};
use betterlite_db::spend_writer::{
    NoSpendWriterFailureSink, SpendWriterFailure, SpendWriterFailureSink, SpendWriterOperation,
};
use betterlite_error::proxy_model_not_found::route_endpoint;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::savings_pricing::context::ModelIdentity;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_cache::connect::{RedisCacheOptions, connect};
use betterlite_redis::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_client_reuse::transport::RedisCacheFamily;
use betterlite_redis::redis_connection_settings::model::scalars::IntOrText;
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use betterlite_redis::redis_live_client::LiveClientFactory;
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::spend::runtime::parts::{SpendRuntimeParts, SpendUsageQueues};
use crate::spend::runtime::pricing::{PriceFuture, PricingView, SpendPricingSource};

// A pricing source over no public rates: identities resolve and every daily row prices at zero.
pub(super) struct TestPricing {
    pub(super) lookup: NoPrices,
    pub(super) zone: TimeZoneDatabase,
}

pub(super) struct NoPrices;

impl PriceLookup for NoPrices {
    fn effective_model_info(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        None
    }

    fn raw_cost_map_entry(&self, _key: &str) -> Option<PriceRecord> {
        None
    }

    fn effective_priced_model(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        None
    }
}

impl SpendPricingSource for TestPricing {
    fn resolve_model(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<ModelIdentity> {
        Some(ModelIdentity {
            model: model.to_owned(),
            provider: custom_llm_provider.unwrap_or_default().to_owned(),
        })
    }

    fn deployment_price(&self, _deployment_id: Option<&str>, _model: &str) -> PriceFuture<'_> {
        Box::pin(ready(None::<PricedModel>))
    }

    fn public_lookup(&self) -> &dyn PriceLookup {
        &self.lookup
    }

    fn zone(&self) -> &TimeZoneDatabase {
        &self.zone
    }
}

pub(super) fn pricing() -> Arc<dyn SpendPricingSource> {
    Arc::new(TestPricing {
        lookup: NoPrices,
        zone: TimeZoneDatabase::none(),
    })
}

pub(super) fn parts_with(pricing: Arc<dyn SpendPricingSource>) -> SpendRuntimeParts {
    SpendRuntimeParts {
        spend_logs: None,
        redis: None,
        pricing,
        pod_id: PodId::try_from(Uuid::new_v4()).expect("a fresh pod id"),
        failures: Arc::new(NoSpendWriterFailureSink),
        pod_lock_events: Arc::new(NoPodLockEventSink),
        queue_events: Arc::new(NoSpendQueueEventSink),
        redis_events: Arc::new(NoRedisBufferEventSink),
        usage: SpendUsageQueues::default(),
    }
}

pub(super) struct StaticPayload(pub(super) &'static str);

impl SpendPayloadSource for StaticPayload {
    type Error = Infallible;

    fn build(&self) -> PayloadFuture<'_, Infallible> {
        Box::pin(ready(Ok(self.0.as_bytes().to_vec())))
    }
}

pub(super) fn request<'call>() -> SpendUpdateRequest<'call, PricingView<'call>> {
    SpendUpdateRequest {
        response_cost: Some(SpendAmount(Decimal::new(25, 2))),
        token: Some("sk-live-token"),
        user_id: Some("u1"),
        end_user_id: None,
        team_id: None,
        org_id: None,
        project_id: None,
        kwargs: None,
        response_names: &[],
        realtime_names: &[],
        model_access_groups: &[],
        turn_environment: None,
    }
}

pub(super) struct RecordingFailures {
    pub(super) reported: Mutex<Vec<SpendWriterOperation>>,
}

impl SpendWriterFailureSink for RecordingFailures {
    fn report(&self, failure: SpendWriterFailure<'_>) {
        self.reported.lock().unwrap().push(failure.operation);
    }
}

pub(super) fn recording_failures() -> (Arc<RecordingFailures>, Arc<dyn SpendWriterFailureSink>) {
    let failures = Arc::new(RecordingFailures {
        reported: Mutex::new(Vec::new()),
    });
    let sink = Arc::clone(&failures);
    (failures, sink)
}

#[derive(Debug, Clone, Copy, thiserror::Error)]
#[error("the scripted spend log service failed")]
pub(super) struct ServiceFailure;

pub(super) struct FakeSpendLogs {
    pub(super) queued: AtomicI64,
    pub(super) runs: AtomicUsize,
}

impl FakeSpendLogs {
    fn clear(&self) -> ServiceFuture<'_, Result<(), ServiceFailure>> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        self.queued.store(0, Ordering::SeqCst);
        Box::pin(ready(Ok(())))
    }
}

impl SpendLogsService for FakeSpendLogs {
    type Error = ServiceFailure;

    fn recover_parked_logs(&self) -> ServiceFuture<'_, usize> {
        Box::pin(ready(0_usize))
    }

    fn queued_transactions(&self) -> i64 {
        self.queued.load(Ordering::SeqCst)
    }

    fn update_spend_logs(&self) -> ServiceFuture<'_, Result<(), ServiceFailure>> {
        self.clear()
    }

    fn run_spend_logs(&self) -> ServiceFuture<'_, Result<(), ServiceFailure>> {
        self.clear()
    }
}

// The call-type map the daily rows record, read through the same table the route error uses.
pub(super) struct Endpoints;

impl CallTypeEndpoints for Endpoints {
    fn endpoint(&self, call_type: &str) -> Option<&str> {
        route_endpoint(call_type)
    }
}

struct QuietHooks;

impl ServiceHooks for QuietHooks {
    fn success(&self, _event: &ServiceEvent) {}

    fn failure(&self, _event: &ServiceEvent, _error: &(dyn StdError + Send + Sync)) {}
}

struct ClosedBreaker;

impl BreakerGate for ClosedBreaker {
    fn enter(&self, _name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Ok(BreakerAdmission::from(0_u64))
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn StdError + Send + Sync)) {}

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn StdError + Send + Sync)) {}
}

// A cache whose address refuses every connection: the Redis handles exist without a server, as the cache routes' own fixture builds one.
pub(super) async fn redis_cache() -> Arc<RedisCache> {
    let mut overrides: RedisOverrides =
        serde_json::from_str("{}").expect("the empty overrides deserialize");
    overrides.host = Some("127.0.0.1".to_owned());
    overrides.port = Some(IntOrText::Value(1_i64));
    let options = RedisCacheOptions {
        overrides,
        family: RedisCacheFamily::Standalone,
        namespace: None,
        default_ttl: 60_i64,
        redis_flush_size: None,
        default_major_version: 7_u32,
        factory: Arc::new(LiveClientFactory::new(None)),
        store: Arc::new(RedisClientStore::new()),
        hooks: Arc::new(QuietHooks),
        gate: Arc::new(ClosedBreaker),
    };
    let environment = BTreeMap::<String, String>::new();
    let cache = connect(options, &environment)
        .await
        .expect("the cache opens without a server");
    Arc::new(cache)
}
