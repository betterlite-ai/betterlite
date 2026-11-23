use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use tokio_util::task::TaskTracker;

use crate::auth_request_models::catalog::{
    ManagedIdDecoders, ManagedIdError, ManagedIdFamily, ModelCatalog,
};
use crate::budget_counter::{BudgetCounter, CounterEntityType};
use crate::reservation_admission::engine::{Admission, AdmissionRequest};
use crate::reservation_admission::error::AdmissionError;
use crate::reservation_cost::EstimateRequest;
use crate::reservation_cost::error::ReservationCostError;
use crate::reservation_counters::CounterInput;
use crate::reservation_counters::error::ReservationCounterError;
use crate::reservation_request::lease::LeaseRenewal;
use crate::reservation_request::ports::{
    ReservationAdmission, ReservationCostSource, ReservationCounterSource, ReservationLeases,
    ReservationSettlement,
};
use crate::reservation_request::{ReservationRequest, ReservationServices};
use crate::reservation_settlement::counters::ScopeHold;
use crate::reservation_settlement::entry::ReservationEntry;
use crate::reservation_settlement::handle::ReservationHandle;
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};

pub(super) const KEY: &str = "spend:key:tok";
pub(super) const ROUTE: &str = "/v1/chat/completions";

pub(super) fn amount(text: &str) -> Decimal {
    text.parse().expect("a fixture decimal")
}

pub(super) fn object(text: &str) -> JsonObject {
    serde_json::from_str(text).expect("the fixture parses")
}

pub(super) fn body() -> JsonObject {
    object(r#"{"model":"gpt-4o"}"#)
}

pub(super) fn auth(text: &str) -> UserAPIKeyAuth {
    let JsonData::Object(fields) =
        serde_json::from_str::<JsonData>(text).expect("the auth fixture")
    else {
        panic!("the auth fixture is a json object");
    };
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("the auth fixture builds")
}

pub(super) fn budget_counter(counter_key: &str, max_budget: &str) -> BudgetCounter {
    BudgetCounter {
        counter_key: counter_key.to_owned(),
        max_budget: amount(max_budget),
        fallback_spend: Decimal::ZERO,
        entity_type: CounterEntityType::Key,
        entity_id: counter_key.to_owned(),
        source_cache_key: Some(counter_key.to_owned()),
        spend_log_entity_id: None,
        window_duration: None,
        window_start: None,
    }
}

fn cloned(counter: &BudgetCounter) -> BudgetCounter {
    BudgetCounter {
        counter_key: counter.counter_key.clone(),
        max_budget: counter.max_budget,
        fallback_spend: counter.fallback_spend,
        entity_type: counter.entity_type.clone(),
        entity_id: counter.entity_id.clone(),
        source_cache_key: counter.source_cache_key.clone(),
        spend_log_entity_id: counter.spend_log_entity_id.clone(),
        window_duration: counter.window_duration.clone(),
        window_start: counter.window_start,
    }
}

pub(super) fn reservation_entry(counter_key: &str, cost: Decimal) -> ReservationEntry {
    ReservationEntry {
        counter_key: Some(counter_key.to_owned()),
        entity_type: CounterEntityType::Key,
        entity_id: counter_key.to_owned(),
        reserved_cost: Some(cost),
        applied_adjustment: Decimal::ZERO,
    }
}

pub(super) fn reservation_handle(counter_keys: &[&str]) -> ReservationHandle {
    let entries = counter_keys
        .iter()
        .map(|counter_key| reservation_entry(counter_key, amount("10")))
        .collect();
    ReservationHandle::new(amount("10"), entries, amount("0"))
}

pub(super) fn reservation_handle_without_keys() -> ReservationHandle {
    let entries = vec![ReservationEntry {
        counter_key: None,
        entity_type: CounterEntityType::Key,
        entity_id: "tok".to_owned(),
        reserved_cost: Some(amount("10")),
        applied_adjustment: Decimal::ZERO,
    }];
    ReservationHandle::new(amount("10"), entries, amount("0"))
}

#[derive(Default)]
pub(super) struct Calls(Mutex<Vec<&'static str>>);

impl Calls {
    pub(super) fn record(&self, call: &'static str) {
        self.0.lock().expect("the call log").push(call);
    }

    pub(super) fn names(&self) -> Vec<&'static str> {
        self.0.lock().expect("the call log").clone()
    }
}

#[derive(Debug)]
pub(super) struct SeenCounter {
    pub(super) tag_names: Vec<String>,
    pub(super) apply_user_budget_to_team_keys: bool,
    pub(super) token_present: bool,
}

pub(super) struct ScriptedCounters {
    pub(super) result: Vec<BudgetCounter>,
    pub(super) seen: Mutex<Vec<SeenCounter>>,
    pub(super) calls: Arc<Calls>,
}

#[async_trait]
impl ReservationCounterSource for ScriptedCounters {
    async fn counters(
        &self,
        input: &CounterInput<'_>,
    ) -> Result<Vec<BudgetCounter>, ReservationCounterError> {
        self.calls.record("counters");
        self.seen
            .lock()
            .expect("the counter log")
            .push(SeenCounter {
                tag_names: input.tag_names.to_vec(),
                apply_user_budget_to_team_keys: input.apply_user_budget_to_team_keys,
                token_present: input.token.view.token.token.is_some(),
            });
        Ok(self.result.iter().map(cloned).collect())
    }
}

pub(super) struct ScriptedCosts {
    pub(super) counts: IndexMap<String, usize>,
    pub(super) max_cost: Option<Decimal>,
    pub(super) input_cost: Option<Decimal>,
    pub(super) calls: Arc<Calls>,
}

#[async_trait]
impl ReservationCostSource for ScriptedCosts {
    async fn input_tokens(
        &self,
        _request: &EstimateRequest<'_>,
    ) -> Result<IndexMap<String, usize>, ReservationCostError> {
        self.calls.record("input_tokens");
        Ok(self.counts.clone())
    }

    async fn max_cost(
        &self,
        _request: &EstimateRequest<'_>,
        _input_token_counts: Option<&IndexMap<String, usize>>,
    ) -> Result<Option<Decimal>, ReservationCostError> {
        Ok(self.recorded("max_cost", self.max_cost))
    }

    async fn input_cost(
        &self,
        _request: &EstimateRequest<'_>,
        _input_token_counts: Option<&IndexMap<String, usize>>,
    ) -> Result<Option<Decimal>, ReservationCostError> {
        Ok(self.recorded("input_cost", self.input_cost))
    }
}

impl ScriptedCosts {
    fn recorded(&self, call: &'static str, cost: Option<Decimal>) -> Option<Decimal> {
        self.calls.record(call);
        cost
    }
}

pub(super) enum AdmissionPlan {
    Applied {
        entries: Vec<(String, Decimal)>,
        reserved_cost: Decimal,
    },
    Empty {
        reserved_cost: Decimal,
    },
    Failed {
        entries: Vec<(String, Decimal)>,
    },
}

#[derive(Debug)]
pub(super) struct SeenAdmission {
    pub(super) reservation_cost: Decimal,
    pub(super) fail_closed_budget_enforcement: bool,
    pub(super) budget_exceeded_status: u16,
    pub(super) throttle_percentage: Option<f64>,
    pub(super) counters: usize,
    pub(super) has_token: bool,
}

pub(super) struct ScriptedAdmission {
    pub(super) plan: AdmissionPlan,
    pub(super) seen: Mutex<Vec<SeenAdmission>>,
    pub(super) calls: Arc<Calls>,
}

fn apply_entries(applied: &mut Vec<ReservationEntry>, entries: &[(String, Decimal)]) {
    for (counter_key, cost) in entries {
        applied.push(reservation_entry(counter_key, *cost));
    }
}

#[async_trait]
impl ReservationAdmission for ScriptedAdmission {
    async fn admit(
        &self,
        request: &AdmissionRequest<'_>,
        applied_entries: &mut Vec<ReservationEntry>,
    ) -> Result<Admission, AdmissionError> {
        self.calls.record("admit");
        self.seen
            .lock()
            .expect("the admission log")
            .push(SeenAdmission {
                reservation_cost: request.reservation_cost,
                fail_closed_budget_enforcement: request.fail_closed_budget_enforcement,
                budget_exceeded_status: request.budget_exceeded_status,
                throttle_percentage: request.throttle_percentage,
                counters: request.counters.len(),
                has_token: request.token.is_some(),
            });
        match &self.plan {
            AdmissionPlan::Applied {
                entries,
                reserved_cost,
            } => {
                apply_entries(applied_entries, entries);
                Ok(Admission {
                    reserved_cost: *reserved_cost,
                })
            }
            AdmissionPlan::Empty { reserved_cost } => Ok(Admission {
                reserved_cost: *reserved_cost,
            }),
            AdmissionPlan::Failed { entries } => {
                apply_entries(applied_entries, entries);
                Err(AdmissionError::Overflow)
            }
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct ReleaseRecord {
    pub(super) entries: Vec<(Option<String>, Option<Decimal>)>,
    pub(super) default_reserved_cost: Decimal,
}

pub(super) struct ScriptedSettlement {
    pub(super) scope_keys: Mutex<Vec<BTreeSet<String>>>,
    pub(super) releases: Mutex<Vec<ReleaseRecord>>,
    pub(super) calls: Arc<Calls>,
}

struct NoScope;

impl ScopeHold for NoScope {}

#[async_trait]
impl ReservationSettlement for ScriptedSettlement {
    fn enter_scope(&self, counter_keys: &BTreeSet<String>) -> Box<dyn ScopeHold + Send + '_> {
        self.calls.record("scope");
        self.scope_keys
            .lock()
            .expect("the scope log")
            .push(counter_keys.clone());
        Box::new(NoScope)
    }

    async fn release_entries(
        &self,
        entries: &mut [ReservationEntry],
        default_reserved_cost: Decimal,
    ) {
        self.calls.record("release");
        let recorded = entries
            .iter()
            .map(|entry| (entry.counter_key.clone(), entry.reserved_cost))
            .collect();
        self.releases
            .lock()
            .expect("the release log")
            .push(ReleaseRecord {
                entries: recorded,
                default_reserved_cost,
            });
    }
}

#[derive(Default)]
pub(super) struct ScriptedLeases {
    refreshed: Mutex<Vec<String>>,
}

#[async_trait]
impl ReservationLeases for ScriptedLeases {
    async fn refresh(&self, counter_key: &str) {
        self.refreshed
            .lock()
            .expect("the refresh log")
            .push(counter_key.to_owned());
    }
}

#[derive(Debug)]
pub(super) struct NoDecoders;

impl ManagedIdDecoders for NoDecoders {
    fn decode_family(
        &self,
        _family: ManagedIdFamily,
        _resource_id: &str,
        _catalog: Option<&dyn ModelCatalog>,
        _candidates: &mut Vec<String>,
    ) -> Result<(), ManagedIdError> {
        Ok(())
    }
}

pub(super) struct World {
    pub(super) calls: Arc<Calls>,
    pub(super) counters: ScriptedCounters,
    pub(super) costs: ScriptedCosts,
    pub(super) admission: ScriptedAdmission,
    pub(super) settlement: ScriptedSettlement,
    pub(super) decoders: NoDecoders,
    pub(super) token: UserAPIKeyAuth,
}

impl World {
    pub(super) fn new() -> Self {
        let calls = Arc::new(Calls::default());
        Self {
            counters: ScriptedCounters {
                result: vec![budget_counter(KEY, "100")],
                seen: Mutex::new(Vec::new()),
                calls: Arc::clone(&calls),
            },
            costs: ScriptedCosts {
                counts: IndexMap::from([("gpt-4o".to_owned(), 7_usize)]),
                max_cost: Some(amount("10")),
                input_cost: Some(amount("3")),
                calls: Arc::clone(&calls),
            },
            admission: ScriptedAdmission {
                plan: AdmissionPlan::Applied {
                    entries: vec![(KEY.to_owned(), amount("10"))],
                    reserved_cost: amount("10"),
                },
                seen: Mutex::new(Vec::new()),
                calls: Arc::clone(&calls),
            },
            settlement: ScriptedSettlement {
                scope_keys: Mutex::new(Vec::new()),
                releases: Mutex::new(Vec::new()),
                calls: Arc::clone(&calls),
            },
            decoders: NoDecoders,
            token: auth(r#"{"token":"tok","max_budget":100.0,"spend":0.0}"#),
            calls,
        }
    }

    pub(super) fn services<'world>(
        &'world self,
        renewal: Option<LeaseRenewal<'world>>,
    ) -> ReservationServices<'world> {
        ReservationServices {
            counters: &self.counters,
            costs: &self.costs,
            admission: &self.admission,
            settlement: &self.settlement,
            renewal,
            budget_exceeded_status: 422,
            throttle_percentage: None,
        }
    }
}

pub(super) struct RequestFixture<'request> {
    body: &'request JsonObject,
    route: &'request str,
    token: Option<&'request UserAPIKeyAuth>,
    decoders: &'request NoDecoders,
    tag_names: &'request [String],
    apply_user_budget_to_team_keys: bool,
    fail_closed_budget_enforcement: bool,
}

impl<'request> RequestFixture<'request> {
    pub(super) fn new(
        body: &'request JsonObject,
        route: &'request str,
        world: &'request World,
    ) -> Self {
        Self {
            body,
            route,
            token: Some(&world.token),
            decoders: &world.decoders,
            tag_names: &[],
            apply_user_budget_to_team_keys: false,
            fail_closed_budget_enforcement: false,
        }
    }

    pub(super) fn without_token(mut self) -> Self {
        self.token = None;
        self
    }

    pub(super) fn with_tag_names(mut self, tag_names: &'request [String]) -> Self {
        self.tag_names = tag_names;
        self
    }

    pub(super) fn fail_closed(mut self) -> Self {
        self.fail_closed_budget_enforcement = true;
        self
    }

    pub(super) fn apply_team_keys(mut self) -> Self {
        self.apply_user_budget_to_team_keys = true;
        self
    }

    pub(super) fn build(&self) -> ReservationRequest<'request> {
        ReservationRequest {
            body: self.body,
            route: self.route,
            catalog: None,
            decoders: self.decoders,
            token: self.token,
            team: None,
            user: None,
            end_user_id: None,
            end_user: None,
            tag_names: self.tag_names,
            apply_user_budget_to_team_keys: self.apply_user_budget_to_team_keys,
            fail_closed_budget_enforcement: self.fail_closed_budget_enforcement,
        }
    }
}

pub(super) struct LeaseHarness {
    pub(super) tracker: TaskTracker,
    recorder: Arc<ScriptedLeases>,
}

impl LeaseHarness {
    pub(super) fn new() -> Self {
        Self {
            tracker: TaskTracker::new(),
            recorder: Arc::new(ScriptedLeases::default()),
        }
    }

    pub(super) fn renewal(
        &self,
        interval: Duration,
        request_timeout: Duration,
    ) -> LeaseRenewal<'_> {
        LeaseRenewal {
            leases: Arc::clone(&self.recorder),
            interval,
            request_timeout,
            tasks: &self.tracker,
        }
    }

    pub(super) fn refreshed(&self) -> Vec<String> {
        self.recorder
            .refreshed
            .lock()
            .expect("the refresh log")
            .clone()
    }
}
