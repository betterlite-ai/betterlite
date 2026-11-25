use std::collections::BTreeSet;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_auth::budget_counter::CounterEntityType;
use betterlite_auth::reservation_settlement::adjust::AdjustmentMode;
use betterlite_auth::reservation_settlement::counters::ScopeHold;
use betterlite_auth::reservation_settlement::entry::ReservationEntry;
use betterlite_auth::reservation_settlement::error::SettlementError;
use betterlite_auth::reservation_settlement::handle::ReservationHandle;
use betterlite_auth::reservation_settlement::reconcile::ReservationFinality;
use betterlite_auth::spend_increments::SpendIncrementInput;
use betterlite_auth::spend_increments::error::SpendIncrementError;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use betterlite_redis::request_batch::batches::RequestRedisBatches;
use betterlite_redis::request_batch::lock;
use betterlite_redis::request_batch::scope::request_redis_batch_scope;
use chrono::{DateTime, TimeZone as _, Utc};
use rust_decimal::Decimal;

use crate::spend_cache_update::SpendCacheUpdateError;
use crate::spend_cache_update::cache::{ArmedSpendRead, SpendCache};

use super::super::error::SpendSettlementError;
use super::super::ports::{
    CounterBatchScope, ReservationSettlement, SpendCounterIncrement, SpendWrite,
};
use super::super::{SettlementInput, SettlementPorts, settle_spend_write};

pub(super) const RESERVED_KEY: &str = "spend:key:reserved";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Fault {
    Reconcile,
    Write,
    NotCharged,
    Release,
    Invalidate,
    Prefetch,
    Increment,
}

// what increment_spend_counters received, owned so the journal outlives the call
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Forwarded {
    pub(super) token: Option<String>,
    pub(super) team_id: Option<String>,
    pub(super) user_id: Option<String>,
    pub(super) org_id: Option<String>,
    pub(super) project_id: Option<String>,
    pub(super) end_user_id: Option<String>,
    pub(super) response_cost: Option<Decimal>,
    pub(super) request_started_at: Option<DateTime<Utc>>,
    pub(super) tags: Option<Vec<JsonData>>,
    pub(super) model_access_groups: Option<Vec<JsonData>>,
    pub(super) reserved: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Effect {
    Reconcile {
        actual_cost: Decimal,
        finality: ReservationFinality,
        mode: AdjustmentMode,
    },
    Invalidate,
    Release,
    BatchOpened(BTreeSet<String>),
    BatchClosed,
    Write,
    Armed(Vec<String>),
    Increment(Box<Forwarded>),
}

#[derive(Debug, thiserror::Error)]
#[error("the scripted spend write failed")]
pub(super) struct WriteFailed;

// every port of the coordinator in one fake, so a single journal orders their effects
pub(super) struct Recorder {
    effects: Mutex<Vec<Effect>>,
    faults: Vec<Fault>,
}

impl Recorder {
    pub(super) fn failing(faults: &[Fault]) -> Self {
        Self {
            effects: Mutex::new(Vec::new()),
            faults: faults.to_vec(),
        }
    }

    pub(super) fn effects(&self) -> Vec<Effect> {
        lock(&self.effects).clone()
    }

    pub(super) async fn settle(
        &self,
        input: &SettlementInput<'_>,
    ) -> Result<bool, SpendSettlementError<WriteFailed>> {
        let ports = SettlementPorts {
            reservations: self,
            batch: self,
            cache: self,
            counters: self,
        };
        let settled = settle_spend_write(input, self, &ports);
        let (_batches, outcome) = request_redis_batch_scope(Duration::from_secs(1), settled).await;
        outcome
    }

    fn record(&self, effect: Effect) {
        lock(&self.effects).push(effect);
    }

    fn scripted(&self, effect: Effect, fault: Fault) -> Result<(), SettlementError> {
        self.record(effect);
        if self.faults.contains(&fault) {
            return Err(SettlementError::Overflow);
        }
        Ok(())
    }
}

struct JournalHold<'recorder> {
    recorder: &'recorder Recorder,
}

impl ScopeHold for JournalHold<'_> {}

impl Drop for JournalHold<'_> {
    fn drop(&mut self) {
        self.recorder.record(Effect::BatchClosed);
    }
}

#[async_trait]
impl ReservationSettlement for Recorder {
    async fn reconcile(
        &self,
        _reservation: &ReservationHandle,
        actual_cost: Decimal,
        finality: ReservationFinality,
        mode: AdjustmentMode,
    ) -> Result<(), SettlementError> {
        let effect = Effect::Reconcile {
            actual_cost,
            finality,
            mode,
        };
        self.scripted(effect, Fault::Reconcile)
    }

    async fn release(&self, _reservation: &ReservationHandle) -> Result<(), SettlementError> {
        self.scripted(Effect::Release, Fault::Release)
    }

    async fn invalidate(&self, _reservation: &ReservationHandle) -> Result<(), SettlementError> {
        self.scripted(Effect::Invalidate, Fault::Invalidate)
    }
}

impl CounterBatchScope for Recorder {
    fn open(&self, counter_keys: &BTreeSet<String>) -> Box<dyn ScopeHold + Send + '_> {
        self.record(Effect::BatchOpened(counter_keys.clone()));
        Box::new(JournalHold { recorder: self })
    }
}

#[async_trait]
impl SpendCache for Recorder {
    fn management_ttl(&self) -> f64 {
        0.0_f64
    }

    fn declare(
        &self,
        keys: &[String],
        _batches: &RequestRedisBatches,
    ) -> Result<Option<ArmedSpendRead>, SpendCacheUpdateError> {
        self.record(Effect::Armed(keys.to_vec()));
        if self.faults.contains(&Fault::Prefetch) {
            return Err(SpendCacheUpdateError::CacheRead);
        }
        Ok(None)
    }

    async fn read(
        &self,
        _keys: &[String],
        _local_only: bool,
    ) -> Result<Vec<Option<Json>>, SpendCacheUpdateError> {
        Ok(Vec::new())
    }

    async fn resolve(
        &self,
        _armed: ArmedSpendRead,
    ) -> Result<Vec<Option<Json>>, SpendCacheUpdateError> {
        Ok(Vec::new())
    }

    async fn cached_key(&self, _key: &str) -> Option<Json> {
        None
    }

    async fn write(&self, _entries: &[(String, Json)], _ttl: f64, _local_only: bool) {}
}

impl SpendWrite for Recorder {
    type Error = WriteFailed;

    async fn update_database(&self) -> Result<bool, WriteFailed> {
        self.record(Effect::Write);
        if self.faults.contains(&Fault::Write) {
            return Err(WriteFailed);
        }
        Ok(!self.faults.contains(&Fault::NotCharged))
    }
}

impl SpendCounterIncrement for Recorder {
    async fn increment(&self, input: &SpendIncrementInput<'_>) -> Result<(), SpendIncrementError> {
        self.record(Effect::Increment(Box::new(Forwarded {
            token: input.token.map(str::to_owned),
            team_id: input.team_id.map(str::to_owned),
            user_id: input.user_id.map(str::to_owned),
            org_id: input.org_id.map(str::to_owned),
            project_id: input.project_id.map(str::to_owned),
            end_user_id: input.end_user_id.map(str::to_owned),
            response_cost: input.response_cost,
            request_started_at: input.request_started_at,
            tags: input.tags.map(<[JsonData]>::to_vec),
            model_access_groups: input.model_access_groups.map(<[JsonData]>::to_vec),
            reserved: input.budget_reservation.is_some(),
        })));
        if self.faults.contains(&Fault::Increment) {
            return Err(SpendIncrementError::Settlement(SettlementError::Overflow));
        }
        Ok(())
    }
}

pub(super) fn started_at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 25, 9, 0, 0)
        .single()
        .expect("a fixture instant")
}

fn texts(value: &str) -> Vec<JsonData> {
    vec![JsonData::String(value.to_owned())]
}

// one request with every attribution field set, so each forwarded value is visible in the journal
pub(super) struct Request {
    pub(super) reservation: ReservationHandle,
    tags: Vec<JsonData>,
    groups: Vec<JsonData>,
    cache_keys: Vec<String>,
}

impl Request {
    pub(super) fn new() -> Self {
        let entry = ReservationEntry {
            counter_key: Some(RESERVED_KEY.to_owned()),
            entity_type: CounterEntityType::Key,
            entity_id: "reserved".to_owned(),
            reserved_cost: Some(Decimal::from(5_i32)),
            applied_adjustment: Decimal::ZERO,
        };
        Self {
            reservation: ReservationHandle::new(Decimal::from(5_i32), vec![entry], Decimal::ZERO),
            tags: texts("tag-a"),
            groups: texts("group-a"),
            cache_keys: vec!["u1".to_owned()],
        }
    }

    pub(super) fn input(&self, reserved: bool) -> SettlementInput<'_> {
        SettlementInput {
            reservation: reserved.then_some(&self.reservation),
            response_cost: Decimal::from(2_i32),
            token: Some("sk-hash"),
            user_id: Some("u1"),
            end_user_id: Some("e1"),
            team_id: Some("t1"),
            org_id: Some("o1"),
            project_id: Some("p1"),
            start_time: Some(started_at()),
            tags: Some(self.tags.as_slice()),
            model_access_groups: Some(self.groups.as_slice()),
            update_cache_read_keys: &self.cache_keys,
        }
    }

    pub(super) fn finalized(&self) -> bool {
        lock(&self.reservation.0).finalized
    }
}

pub(super) fn reconciled() -> Effect {
    Effect::Reconcile {
        actual_cost: Decimal::from(2_i32),
        finality: ReservationFinality::LeaveOpen,
        mode: AdjustmentMode::Defer,
    }
}

// B1718's post-call keys for the request, project and access group included, plus the reserved counter
pub(super) fn opened(reserved: bool) -> Effect {
    let mut keys = BTreeSet::from(
        [
            "spend:key:sk-hash",
            "spend:team:t1",
            "spend:team_member:u1:t1",
            "spend:user:u1",
            "spend:end_user:e1",
            "spend:org:o1",
            "spend:project:p1",
            "spend:tag:tag-a",
            "spend:model_access_group:group-a",
        ]
        .map(str::to_owned),
    );
    if reserved {
        let _added = keys.insert(RESERVED_KEY.to_owned());
    }
    Effect::BatchOpened(keys)
}

pub(super) fn armed() -> Effect {
    Effect::Armed(vec!["u1".to_owned()])
}

pub(super) fn forwarded(reserved: bool) -> Effect {
    Effect::Increment(Box::new(Forwarded {
        token: Some("sk-hash".to_owned()),
        team_id: Some("t1".to_owned()),
        user_id: Some("u1".to_owned()),
        org_id: Some("o1".to_owned()),
        project_id: Some("p1".to_owned()),
        end_user_id: Some("e1".to_owned()),
        response_cost: Some(Decimal::from(2_i32)),
        request_started_at: Some(started_at()),
        tags: Some(texts("tag-a")),
        model_access_groups: Some(texts("group-a")),
        reserved,
    }))
}
