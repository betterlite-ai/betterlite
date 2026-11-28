#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode};
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, Role};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::sections::settings::JsonValue;
use betterlite_core::api::anthropic_beta_config::matrix::{BetaMatrix, BetaNode};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::reload_schedule::ReloadSchedule;
use betterlite_db::reload_schedule::store::ReloadScheduleError;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::catalog_generalizations::CatalogGeneralizations;
use betterlite_models::fetch_policy::outcome::Unavailable;
use betterlite_models::reload::ReloadOutcome;
use betterlite_models::source::{CatalogProvenance, CatalogSnapshot, SourceKind};
use chrono::{DateTime, TimeDelta, TimeZone as _, Utc};
use indexmap::IndexMap;
use tower::ServiceExt as _;

use crate::auth::auth_identity::AuthContext;
use crate::auth::auth_key_lookup::tests::master_caller;
use crate::periodic_reloads::state::PeriodicReloads;
use crate::reload_routes::ports::{
    BetaHeadersReload, BetaReloadFuture, CatalogReload, CatalogReloadFuture, ReloadConfigStore,
    ReloadStoreError, StoreFuture,
};
use crate::reload_routes::{ReloadRoutesState, routes};

pub(crate) const COST_MAP_RELOAD: &str = "/reload/model_cost_map";
pub(crate) const COST_MAP_SCHEDULE: &str = "/schedule/model_cost_map_reload";
pub(crate) const COST_MAP_STATUS: &str = "/schedule/model_cost_map_reload/status";
pub(crate) const COST_MAP_SOURCE: &str = "/model/cost_map/source";
pub(crate) const BETA_RELOAD: &str = "/reload/anthropic_beta_headers";
pub(crate) const BETA_SCHEDULE: &str = "/schedule/anthropic_beta_headers_reload";
pub(crate) const BETA_STATUS: &str = "/schedule/anthropic_beta_headers_reload/status";
pub(crate) const COST_MAP_SCHEDULE_2_HOURS: &str = "/schedule/model_cost_map_reload?hours=2";
pub(crate) const BETA_SCHEDULE_2_HOURS: &str = "/schedule/anthropic_beta_headers_reload?hours=2";
pub(crate) const WRITES: [(Method, &str); 6] = [
    (Method::POST, COST_MAP_RELOAD),
    (Method::POST, COST_MAP_SCHEDULE_2_HOURS),
    (Method::DELETE, COST_MAP_SCHEDULE),
    (Method::POST, BETA_RELOAD),
    (Method::POST, BETA_SCHEDULE_2_HOURS),
    (Method::DELETE, BETA_SCHEDULE),
];
pub(crate) const READS: [(Method, &str); 3] = [
    (Method::GET, COST_MAP_STATUS),
    (Method::GET, COST_MAP_SOURCE),
    (Method::GET, BETA_STATUS),
];
pub(crate) const FORBIDDEN_ROLE: &str = "Access denied. Admin role required. Current role:";
pub(crate) const STORE_FAILURE: &str = "the published reload revision -1 does not fit a u32";

pub(crate) fn at(hour: u32, minute: u32, second: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 23, hour, minute, second)
        .unwrap()
}

pub(crate) fn at_year(year: i32, month: u32, day: u32, hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, hour, 0, 0).unwrap()
}

pub(crate) fn moment() -> DateTime<Utc> {
    at(12, 0, 0)
}

pub(crate) fn one_hour_before_datetime_max() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(9999, 12, 31, 22, 59, 59)
        .unwrap()
        .checked_add_signed(TimeDelta::microseconds(999_999))
        .unwrap()
}

pub(crate) fn with_micros(base: DateTime<Utc>, micros: i64) -> DateTime<Utc> {
    base.checked_add_signed(TimeDelta::microseconds(micros))
        .unwrap()
}

pub(crate) fn parsed(raw: &str) -> JsonValue {
    serde_json::from_str(raw).unwrap()
}

pub(crate) fn json_data(raw: &str) -> JsonData {
    serde_json::from_str(raw).unwrap()
}

pub(crate) fn local_provenance() -> CatalogProvenance {
    CatalogProvenance {
        kind: SourceKind::Local,
        url: None,
        is_env_forced: false,
        fallback_reason: None,
        loaded_at: None,
        source_revision: None,
        etag: None,
    }
}

pub(crate) fn env_forced_provenance() -> CatalogProvenance {
    let mut provenance = local_provenance();
    provenance.is_env_forced = true;
    provenance
}

pub(crate) fn provenance_loaded_at(loaded_at: DateTime<Utc>) -> CatalogProvenance {
    CatalogProvenance {
        kind: SourceKind::Local,
        url: None,
        is_env_forced: false,
        fallback_reason: None,
        loaded_at: Some(loaded_at),
        source_revision: None,
        etag: None,
    }
}

pub(crate) fn reloaded_provenance() -> CatalogProvenance {
    let mut provenance = local_provenance();
    provenance.kind = SourceKind::Remote;
    provenance.url = Some("https://example.test/cost.json".to_owned());
    provenance.loaded_at = Some(moment());
    provenance.source_revision = Some("rev-1".to_owned());
    provenance.etag = Some("etag-1".to_owned());
    provenance
}

pub(crate) fn snapshot(provenance: CatalogProvenance, adopted_size: usize) -> Arc<CatalogSnapshot> {
    let catalog = ModelCatalog {
        entries: BTreeMap::new(),
        lookup: BTreeMap::new(),
        generalizations: CatalogGeneralizations::default(),
        warnings: Vec::new(),
        revision: None,
        adopted_size,
    };
    Arc::new(CatalogSnapshot {
        catalog: Arc::new(catalog),
        provenance,
    })
}

pub(crate) fn beta_matrix() -> BetaMatrix {
    let mut map = IndexMap::new();
    map.insert("anthropic".to_owned(), BetaNode::Map(IndexMap::new()));
    map.insert("bedrock".to_owned(), BetaNode::Map(IndexMap::new()));
    map.insert(
        "provider_aliases".to_owned(),
        BetaNode::Map(IndexMap::new()),
    );
    map.insert("description".to_owned(), BetaNode::Text("beta".to_owned()));
    BetaMatrix(map)
}

#[derive(Clone, Default)]
pub(crate) struct FakeStore {
    state: Arc<Mutex<StoreState>>,
}

#[derive(Clone, Default)]
pub(crate) struct StoreState {
    pub(crate) schedule: Option<ReloadSchedule>,
    pub(crate) beta: Option<JsonData>,
    pub(crate) revision: u32,
    pub(crate) written_intervals: Vec<u32>,
    pub(crate) cleared: usize,
    pub(crate) recorded: Vec<DateTime<Utc>>,
    pub(crate) beta_writes: Vec<JsonData>,
    pub(crate) beta_deletes: usize,
    pub(crate) beta_invalidations: usize,
    fail: bool,
}

fn store_failure() -> ReloadStoreError {
    ReloadStoreError::Schedule(ReloadScheduleError::Revision { value: -1 })
}

fn empty_state() -> StoreState {
    StoreState {
        schedule: None,
        beta: None,
        revision: 0,
        written_intervals: Vec::new(),
        cleared: 0,
        recorded: Vec::new(),
        beta_writes: Vec::new(),
        beta_deletes: 0,
        beta_invalidations: 0,
        fail: false,
    }
}

impl FakeStore {
    fn from_state(state: StoreState) -> Self {
        Self {
            state: Arc::new(Mutex::new(state)),
        }
    }

    pub(crate) fn failing() -> Self {
        let mut state = empty_state();
        state.fail = true;
        Self::from_state(state)
    }

    pub(crate) fn with_schedule(schedule: ReloadSchedule) -> Self {
        let mut state = empty_state();
        state.schedule = Some(schedule);
        Self::from_state(state)
    }

    pub(crate) fn with_beta(value: JsonData) -> Self {
        let mut state = empty_state();
        state.beta = Some(value);
        Self::from_state(state)
    }

    pub(crate) fn with_revision(revision: u32) -> Self {
        let mut state = empty_state();
        state.revision = revision;
        Self::from_state(state)
    }

    pub(crate) fn snapshot(&self) -> StoreState {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record<Value, Change>(&self, change: Change) -> StoreFuture<'_, Value>
    where
        Value: Send + 'static,
        Change: FnOnce(&mut StoreState) -> Value,
    {
        let answer = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if state.fail {
                Err(store_failure())
            } else {
                Ok(change(&mut state))
            }
        };
        Box::pin(async move { answer })
    }
}

impl ReloadConfigStore for FakeStore {
    fn read_schedule(&self) -> StoreFuture<'_, Option<ReloadSchedule>> {
        self.record(|state| state.schedule.clone())
    }

    fn write_interval(&self, interval_hours: u32) -> StoreFuture<'_, ()> {
        self.record(|state| {
            state.written_intervals.push(interval_hours);
            state.schedule = Some(ReloadSchedule {
                interval_hours: Some(interval_hours),
                reload_revision: state.revision,
                last_run_at: None,
            });
        })
    }

    fn clear_interval(&self) -> StoreFuture<'_, ()> {
        self.record(|state| {
            state.cleared = state.cleared.saturating_add(1);
            if let Some(schedule) = state.schedule.as_mut() {
                schedule.interval_hours = None;
            }
        })
    }

    fn record_manual_reload(&self, ran_at: DateTime<Utc>) -> StoreFuture<'_, u32> {
        self.record(|state| {
            state.recorded.push(ran_at);
            state.revision
        })
    }

    fn read_beta_config(&self) -> StoreFuture<'_, Option<JsonData>> {
        self.record(|state| state.beta.clone())
    }

    fn write_beta_config(&self, value: JsonData) -> StoreFuture<'_, ()> {
        self.record(|state| {
            state.beta_writes.push(value.clone());
            state.beta = Some(value);
        })
    }

    fn delete_beta_config(&self) -> StoreFuture<'_, ()> {
        self.record(|state| {
            state.beta_deletes = state.beta_deletes.saturating_add(1);
            state.beta = None;
        })
    }

    fn invalidate_beta_config(&self) -> StoreFuture<'_, ()> {
        self.record(|state| {
            state.beta_invalidations = state.beta_invalidations.saturating_add(1);
        })
    }
}

#[derive(Clone)]
pub(crate) struct FakeCatalog {
    state: Arc<Mutex<CatalogState>>,
}

struct CatalogState {
    current: Arc<CatalogSnapshot>,
    outcome: ReloadOutcome,
    reloads: usize,
}

impl FakeCatalog {
    pub(crate) fn reloaded(snapshot: Arc<CatalogSnapshot>) -> Self {
        Self::new(Arc::clone(&snapshot), ReloadOutcome::Reloaded(snapshot))
    }

    pub(crate) fn with_current(current: Arc<CatalogSnapshot>) -> Self {
        let outcome = ReloadOutcome::Reloaded(Arc::clone(&current));
        Self::new(current, outcome)
    }

    pub(crate) fn unavailable(reason: &str) -> Self {
        let current = snapshot(local_provenance(), 4);
        let outcome = ReloadOutcome::Unavailable(Unavailable {
            reason: reason.to_owned(),
        });
        Self::new(current, outcome)
    }

    fn new(current: Arc<CatalogSnapshot>, outcome: ReloadOutcome) -> Self {
        Self {
            state: Arc::new(Mutex::new(CatalogState {
                current,
                outcome,
                reloads: 0,
            })),
        }
    }

    pub(crate) fn reloads(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .reloads
    }

    pub(crate) fn adopted(&self) -> Arc<CatalogSnapshot> {
        Arc::clone(
            &self
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .current,
        )
    }
}

impl CatalogReload for FakeCatalog {
    fn current(&self) -> Arc<CatalogSnapshot> {
        self.adopted()
    }

    fn reload(&self) -> CatalogReloadFuture<'_> {
        let outcome = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.reloads = state.reloads.saturating_add(1);
            let outcome = state.outcome.clone();
            if let ReloadOutcome::Reloaded(snapshot) = &outcome {
                state.current = Arc::clone(snapshot);
            }
            outcome
        };
        Box::pin(async move { outcome })
    }
}

#[derive(Clone)]
pub(crate) struct FakeBeta {
    matrix: BetaMatrix,
    reloads: Arc<AtomicUsize>,
}

impl FakeBeta {
    pub(crate) fn new(matrix: BetaMatrix) -> Self {
        Self {
            matrix,
            reloads: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub(crate) fn reloads(&self) -> usize {
        self.reloads.load(Ordering::SeqCst)
    }
}

impl BetaHeadersReload for FakeBeta {
    fn reload(&self) -> BetaReloadFuture<'_> {
        self.reloads.fetch_add(1, Ordering::SeqCst);
        let matrix = Arc::new(self.matrix.clone());
        Box::pin(async move { matrix })
    }
}

pub(crate) struct Harness {
    pub(crate) store: Option<FakeStore>,
    pub(crate) catalog: FakeCatalog,
    pub(crate) beta: FakeBeta,
    pub(crate) reloads: Arc<PeriodicReloads>,
    pub(crate) now: DateTime<Utc>,
}

impl Harness {
    pub(crate) fn new() -> Self {
        let store = Some(FakeStore::default());
        let catalog = FakeCatalog::reloaded(snapshot(reloaded_provenance(), 3));
        let beta = FakeBeta::new(beta_matrix());
        let reloads = Arc::new(PeriodicReloads::new(
            &CatalogProvenance::default(),
            moment(),
        ));
        Self {
            store,
            catalog,
            beta,
            reloads,
            now: moment(),
        }
    }

    pub(crate) fn store(&self) -> &FakeStore {
        self.store.as_ref().expect("a fake store")
    }

    fn state(&self) -> ReloadRoutesState {
        let config: Option<Arc<dyn ReloadConfigStore>> = self
            .store
            .as_ref()
            .map(|fake| -> Arc<dyn ReloadConfigStore> { Arc::new(fake.clone()) });
        let now = self.now;
        ReloadRoutesState {
            config,
            catalog: Arc::new(self.catalog.clone()),
            beta_headers: Arc::new(self.beta.clone()),
            reloads: Arc::clone(&self.reloads),
            clock: Arc::new(move || now),
        }
    }
}

pub(crate) async fn send(
    harness: &Harness,
    who: Option<AuthContext>,
    method: Method,
    uri: &str,
) -> (StatusCode, String) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(context) = who {
        builder = builder.extension(context);
    }
    let request = builder.body(Body::empty()).unwrap();
    let response = routes(harness.state()).oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

pub(crate) async fn every_route_answer(
    harness: &Harness,
    who: &AuthContext,
    routes: &[(Method, &str)],
    denied_role: Option<&str>,
) {
    for (method, uri) in routes {
        let (status, text) = send(harness, Some(who.clone()), method.clone(), uri).await;
        match denied_role {
            Some(role) => {
                assert_eq!(status, StatusCode::FORBIDDEN, "{uri}");
                assert_eq!(parsed(&text)["detail"], format!("{FORBIDDEN_ROLE} {role}"));
            }
            None => assert_eq!(status, StatusCode::OK, "{uri} {text}"),
        }
    }
}

pub(crate) fn context(subject: &str, roles: &[Role], source_key: SourceKey) -> AuthContext {
    AuthContext {
        principal: Principal {
            principal_type: PrincipalType::Human,
            subject: subject.to_owned(),
            issuer: None,
            audience: Vec::new(),
            user: None,
            organization: None,
            teams: Vec::new(),
            project: None,
            end_user: None,
            roles: roles.to_vec(),
            scopes: Vec::new(),
            auth_method: AuthMethod::ApiKey,
            credential_ref: CredentialRef::default(),
            network: NetworkContext::default(),
            source_key,
        },
    }
}

pub(crate) fn master() -> AuthContext {
    master_caller()
}

pub(crate) fn platform_admin() -> AuthContext {
    context("jwt-user", &[Role::PlatformAdmin], SourceKey(None))
}

pub(crate) fn platform_viewer() -> AuthContext {
    context("jwt-user", &[Role::PlatformViewer], SourceKey(None))
}

pub(crate) fn key_caller(fields: &str) -> AuthContext {
    let decoded: BTreeMap<String, JsonData> = serde_json::from_str(fields).unwrap();
    let key = UserAPIKeyAuth::construct(AuthInput::Dict(decoded)).unwrap();
    context("key-owner", &[], SourceKey(Some(key)))
}

pub(crate) fn key_with(role: &str) -> AuthContext {
    key_caller(&format!(r#"{{"user_role": "{role}"}}"#))
}
