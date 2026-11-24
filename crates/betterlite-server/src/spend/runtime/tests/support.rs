#![cfg(test)]

use std::convert::Infallible;
use std::future::ready;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

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
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::savings_pricing::context::ModelIdentity;
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

pub(super) fn request<'call>(
    view: &'call PricingView<'call>,
) -> SpendUpdateRequest<'call, PricingView<'call>> {
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

#[derive(Debug, Clone, Copy, thiserror::Error)]
#[error("the scripted spend log service failed")]
pub(super) struct ServiceFailure;

pub(super) struct FakeSpendLogs {
    pub(super) queued: AtomicI64,
    pub(super) runs: AtomicUsize,
}

impl FakeSpendLogs {
    async fn clear(&self) -> Result<(), ServiceFailure> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        self.queued.store(0, Ordering::SeqCst);
        Ok(())
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
        Box::pin(self.clear())
    }

    fn run_spend_logs(&self) -> ServiceFuture<'_, Result<(), ServiceFailure>> {
        Box::pin(self.clear())
    }
}
