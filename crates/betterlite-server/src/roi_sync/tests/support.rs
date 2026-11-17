#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use axum::http::HeaderMap;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_models::model_capability_predicates::{CapabilityPredicates, CapabilityRequest};
use betterlite_roi::estimator::cache::cache_context;
use betterlite_roi::estimator::models::ROICompletionRequest;
use betterlite_roi::records::{
    ROIEstimate, ROIEstimateStatus, ROIPullRecord, ROIReport, ROISpendRecord, ROISyncPhase,
    ROISyncStatus,
};
use betterlite_roi::settings::ROISettings;
use chrono::{DateTime, NaiveDate, Utc};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::de::DeserializeOwned;
use serde_json::value::RawValue;
use thiserror::Error;
use tokio::sync::{Barrier, Notify};
use tokio::task::yield_now;
use uuid::Uuid;

use crate::roi_cache::key::cache_key;
use crate::roi_cache::store::{PULL_PARAM_PREFIX, REPORT_PARAM, ReportRepository};
use crate::roi_estimator::{CompletionCaller, CompletionFuture, CompletionObject};
use crate::roi_github::client::GitHub;
use crate::roi_github::error::SourceError;
use crate::roi_github::models::GitHubPullListItem;
use crate::roi_github::transport::Transport;
use crate::roi_spend::SpendReader;
use crate::roi_sync::capabilities::{CapabilityLookup, SessionCapabilities};
use crate::roi_sync::coordinator::SyncCoordinator;
use crate::roi_sync::manager::{SyncManager, SyncRun};
use crate::roi_sync::ports::{GitHubFactory, SyncClock, UuidSource};

pub(super) const ANSWER: &str = r#"{"hours": 3.5, "reasoning": "Small change."}"#;

pub(super) fn wire<Wire>(text: &str) -> Wire
where
    Wire: DeserializeOwned,
{
    serde_json::from_str(text).unwrap()
}

pub(super) async fn until<Condition>(mut condition: Condition)
where
    Condition: FnMut() -> bool,
{
    for _ in 0_u32..20_000 {
        if condition() {
            return;
        }
        yield_now().await;
    }
    panic!("the condition never held");
}

#[derive(Clone, Default)]
pub(super) struct Routes {
    answers: Arc<Mutex<BTreeMap<String, (u16, String)>>>,
    gates: Arc<Mutex<BTreeMap<String, Arc<Notify>>>>,
    requested: Arc<Mutex<Vec<String>>>,
}

impl Routes {
    pub(super) fn with(self, path: &str, status: u16, body: &str) -> Self {
        self.replace(path, status, body);
        self
    }

    pub(super) fn replace(&self, path: &str, status: u16, body: &str) {
        self.answers
            .lock()
            .unwrap()
            .insert(path.to_owned(), (status, body.to_owned()));
    }

    pub(super) fn merge(self, other: &Self) -> Self {
        let borrowed = other.answers.lock().unwrap().clone();
        self.answers.lock().unwrap().extend(borrowed);
        let borrowed_gates = other.gates.lock().unwrap().clone();
        self.gates.lock().unwrap().extend(borrowed_gates);
        self
    }

    pub(super) fn gate(&self, path: &str) -> Arc<Notify> {
        let gate = Arc::new(Notify::new());
        self.gates
            .lock()
            .unwrap()
            .insert(path.to_owned(), Arc::clone(&gate));
        gate
    }

    pub(super) fn requested(&self) -> Vec<String> {
        self.requested.lock().unwrap().clone()
    }
}

impl Transport for Routes {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let path = request.url.path().to_owned();
        let answers = Arc::clone(&self.answers);
        let gates = Arc::clone(&self.gates);
        let requested = Arc::clone(&self.requested);
        async move {
            requested.lock().unwrap().push(path.clone());
            let waiting = gates.lock().unwrap().get(&path).cloned();
            if let Some(gate) = waiting {
                gate.notified().await;
            }
            let (status, body) = answers
                .lock()
                .unwrap()
                .get(&path)
                .cloned()
                .unwrap_or((404, String::new()));
            Ok(UpstreamResponse {
                status,
                headers: HeaderMap::new(),
                body: body.into_bytes(),
            })
        }
    }
}

pub(super) fn factory() -> GitHubFactory<Routes> {
    Arc::new(|settings, transport| GitHub::new(settings, transport.unwrap_or_default()))
}

pub(super) fn failing_factory() -> GitHubFactory<Routes> {
    Arc::new(|_settings, _transport| Err(SourceError::PaginationLimit))
}

pub(super) fn settings(repos: &[&str]) -> ROISettings {
    ROISettings {
        github_api_url: "https://api.github.com".to_owned(),
        github_token: SecretString::default(),
        estimator_key: SecretString::default(),
        repos: repos.iter().map(|repo| (*repo).to_owned()).collect(),
        estimator_model: "gpt-5.2".to_owned(),
        estimator_prompt: "Estimate this.".to_owned(),
        backfill_days: 7,
        update_interval_minutes: 1440.0_f64,
        identity_map: IndexMap::new(),
    }
}

const FIXED_NOW: &str = "2026-01-10T00:00:00Z";

#[derive(Clone)]
pub(super) struct TestClock {
    at: Arc<Mutex<DateTime<Utc>>>,
}

impl TestClock {
    pub(super) fn at(text: &str) -> Self {
        Self {
            at: Arc::new(Mutex::new(text.parse().unwrap())),
        }
    }

    pub(super) fn set(&self, text: &str) {
        *self.at.lock().unwrap() = text.parse().unwrap();
    }

    pub(super) fn handle(&self) -> SyncClock {
        let at = Arc::clone(&self.at);
        let clock: SyncClock = Arc::new(move || *at.lock().unwrap_or_else(PoisonError::into_inner));
        clock
    }
}

pub(super) fn pull(number: i64) -> GitHubPullListItem {
    wire(&pull_item_json(number))
}

fn pull_item_json(number: i64) -> String {
    format!(
        r#"{{"number":{number},"html_url":"https://github.com/o/r/pull/{number}","merged_at":"2026-01-05T00:00:00Z","updated_at":"2026-01-06T00:00:00Z","title":"Pull {number}","body":"Body","head":{{"sha":"sha{number}"}},"user":{{"login":"dev"}}}}"#
    )
}

fn pull_detail_json(number: i64) -> String {
    format!(
        r#"{{"number":{number},"title":"Pull {number}","body":"Body","html_url":"https://github.com/o/r/pull/{number}","user":{{"login":"dev"}},"merged_at":"2026-01-05T00:00:00Z","head":{{"sha":"sha{number}"}},"additions":10,"deletions":2,"changed_files":1,"commits":1}}"#
    )
}

const PULL_FILES: &str = r#"[{"filename":"a.py","status":"added","additions":10,"deletions":2}]"#;
const PULL_COMMITS: &str = r#"[{"sha":"c1","author":{"login":"dev"},"commit":{"message":"first","author":{"email":"work@example.com"}}}]"#;
const PROFILE: &str = r#"{"email":"dev@example.com"}"#;

fn list_json(numbers: &[i64]) -> String {
    let items: Vec<String> = numbers
        .iter()
        .map(|number| pull_item_json(*number))
        .collect();
    format!("[{}]", items.join(","))
}

pub(super) fn routes_for(repo: &str, numbers: &[i64]) -> Routes {
    let mut routes = Routes::default()
        .with(&format!("/repos/{repo}/pulls"), 200, &list_json(numbers))
        .with("/users/dev", 200, PROFILE);
    for number in numbers {
        for (path, body) in pull_routes(repo, *number) {
            routes = routes.with(&path, 200, &body);
        }
    }
    routes
}

fn pull_routes(repo: &str, number: i64) -> [(String, String); 3] {
    [
        (
            format!("/repos/{repo}/pulls/{number}"),
            pull_detail_json(number),
        ),
        (
            format!("/repos/{repo}/pulls/{number}/files"),
            PULL_FILES.to_owned(),
        ),
        (
            format!("/repos/{repo}/pulls/{number}/commits"),
            PULL_COMMITS.to_owned(),
        ),
    ]
}

pub(super) async fn pull_key(
    settings: &ROISettings,
    repo: &str,
    item: &GitHubPullListItem,
) -> String {
    let empty = NoDeclaredCapabilities;
    let mut predicates = SessionCapabilities { lookup: &empty };
    let context = cache_context(settings, None, &mut predicates)
        .await
        .unwrap();
    cache_key(settings, &context, repo, item).unwrap().unwrap()
}

pub(super) fn pull_param(key: &str) -> String {
    format!("{PULL_PARAM_PREFIX}{key}")
}

pub(super) fn record_text(number: i64, key: Option<&str>) -> String {
    let cache_key = key.map_or_else(|| "null".to_owned(), |value| format!("\"{value}\""));
    format!(
        r#"{{"repo":"o/r","number":{number},"title":"Pull {number}","url":"https://github.com/o/r/pull/{number}","login":"dev","emails":["work@example.com","old@example.com"],"profile_email":"old@example.com","commit_emails":["work@example.com"],"merged_at":"2026-01-05T00:00:00Z","head_sha":"sha{number}","additions":10,"deletions":2,"changed_files":1,"commit_count":1,"incomplete_metadata":false,"estimate":{{"status":"estimated","hours":2.0,"reasoning":"Cached.","cached":true}},"cache_key":{cache_key}}}"#
    )
}

pub(super) fn report_text(numbers: &[(i64, &str)]) -> String {
    let pulls: Vec<String> = numbers
        .iter()
        .map(|(number, key)| record_text(*number, Some(key)))
        .collect();
    format!(
        r#"{{"mode":"live","start":"2026-01-04","end":"2026-01-10","synced_at":"2026-01-10T00:00:00+00:00","repos":["o/r"],"estimator_model":"gpt-5.2","estimator_prompt":"Estimate this.","effort_basis":"without_ai","spend":[],"pulls":[{}],"settings_fingerprint":"old"}}"#,
        pulls.join(",")
    )
}

pub(super) fn unchanged_record_text(number: i64, key: &str) -> String {
    record_text(number, Some(key)).replace(
        r#""emails":["work@example.com","old@example.com"]"#,
        r#""emails":["old@example.com","work@example.com"]"#,
    )
}

pub(super) fn unchanged_report_text(numbers: &[(i64, &str)]) -> String {
    report_text(numbers).replace(
        r#""emails":["work@example.com","old@example.com"]"#,
        r#""emails":["old@example.com","work@example.com"]"#,
    )
}

pub(super) fn report_of(repository: &MemoryRepository) -> ROIReport {
    let value = repository
        .written_named(REPORT_PARAM)
        .expect("the report was stored");
    wire(&serde_json::to_string(&value).unwrap())
}

pub(super) fn written_record(repository: &MemoryRepository, name: &str) -> ROIPullRecord {
    let value = repository
        .written_named(name)
        .expect("the pull record was stored");
    wire(&serde_json::to_string(&value).unwrap())
}

pub(super) fn blank_status() -> ROISyncStatus {
    ROISyncStatus {
        running: false,
        phase: ROISyncPhase::Idle,
        stage: "Idle".to_owned(),
        done: 0,
        total: 0,
        estimated: 0,
        reused: 0,
        needs_attention: 0,
        error: None,
        started_at: None,
        finished_at: None,
        next_update: None,
        elapsed_seconds: 0,
        remaining_seconds: None,
    }
}

pub(super) fn estimate(status: ROIEstimateStatus, cached: Option<bool>) -> ROIEstimate {
    ROIEstimate {
        status,
        hours: None,
        reasoning: String::new(),
        model: None,
        evidence_source: None,
        effort_basis: None,
        cached,
    }
}

pub(super) fn spend_row() -> ROISpendRecord {
    wire(
        r#"{"date":"2026-01-05","user_id":"user-1","email":"dev@example.com","spend":1.5,"requests":2}"#,
    )
}

#[derive(Debug, Error)]
pub(super) enum MemoryError {
    #[error("the memory repository refused the write")]
    Refused,
    #[error("the memory repository refused the read")]
    Unreadable,
}

#[derive(Default)]
struct MemoryState {
    params: BTreeMap<String, JsonData>,
    writes: Vec<(String, JsonData)>,
    refuse_reads: bool,
    refuse_writes: bool,
}

#[derive(Clone, Default)]
pub(super) struct MemoryRepository {
    state: Arc<Mutex<MemoryState>>,
}

impl MemoryRepository {
    pub(super) fn holding_text(name: &str, text: &str) -> Self {
        let repository = Self::default();
        repository.insert_param(name, text);
        repository
    }

    pub(super) fn with_param(self, name: &str, text: &str) -> Self {
        self.insert_param(name, text);
        self
    }

    fn insert_param(&self, name: &str, text: &str) {
        self.state
            .lock()
            .unwrap()
            .params
            .insert(name.to_owned(), wire(text));
    }

    pub(super) fn refusing_writes() -> Self {
        let repository = Self::default();
        repository.state.lock().unwrap().refuse_writes = true;
        repository
    }

    pub(super) fn refusing_reads() -> Self {
        let repository = Self::default();
        repository.state.lock().unwrap().refuse_reads = true;
        repository
    }

    pub(super) fn written(&self) -> Vec<(String, JsonData)> {
        self.state.lock().unwrap().writes.clone()
    }

    pub(super) fn written_named(&self, name: &str) -> Option<JsonData> {
        self.state
            .lock()
            .unwrap()
            .writes
            .iter()
            .rev()
            .find(|(written, _)| written == name)
            .map(|(_, value)| value.clone())
    }

    fn read(&self, name: &str) -> Result<Option<JsonData>, MemoryError> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.refuse_reads {
            return Err(MemoryError::Unreadable);
        }
        Ok(state.params.get(name).cloned())
    }

    fn write(&self, name: &str, value: JsonData) -> Result<(), MemoryError> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.refuse_writes {
            return Err(MemoryError::Refused);
        }
        state.writes.push((name.to_owned(), value.clone()));
        state.params.insert(name.to_owned(), value);
        drop(state);
        Ok(())
    }
}

impl ReportRepository for MemoryRepository {
    type Error = MemoryError;

    fn get_param(
        &self,
        param_name: &str,
    ) -> impl Future<Output = Result<Option<JsonData>, MemoryError>> + Send {
        ready(self.read(param_name))
    }

    fn set_param(
        &self,
        param_name: &str,
        param_value: JsonData,
    ) -> impl Future<Output = Result<(), MemoryError>> + Send {
        ready(self.write(param_name, param_value))
    }
}

#[derive(Default)]
struct SpendState {
    ranges: Vec<(NaiveDate, NaiveDate)>,
    rows: Vec<ROISpendRecord>,
    fail: bool,
}

#[derive(Clone, Default)]
pub(super) struct FakeSpend {
    state: Arc<Mutex<SpendState>>,
}

impl FakeSpend {
    pub(super) fn with_rows(rows: Vec<ROISpendRecord>) -> Self {
        let spend = Self::default();
        spend.state.lock().unwrap().rows = rows;
        spend
    }

    pub(super) fn failing() -> Self {
        let spend = Self::default();
        spend.state.lock().unwrap().fail = true;
        spend
    }

    pub(super) fn ranges(&self) -> Vec<(NaiveDate, NaiveDate)> {
        self.state.lock().unwrap().ranges.clone()
    }
}

impl SpendReader for FakeSpend {
    fn read(
        &self,
        start: NaiveDate,
        end: NaiveDate,
    ) -> impl Future<Output = Result<Vec<ROISpendRecord>, DbError>> + Send {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.ranges.push((start, end));
        let rows = state.rows.clone();
        let outcome = if state.fail {
            Err(DbError::NoDbConnected)
        } else {
            Ok(rows)
        };
        drop(state);
        ready(outcome)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Lease {
    Acquire,
    Heartbeat,
    Finish,
}

#[derive(Default)]
pub(super) struct CoordinatorState {
    pub(super) shared: Option<ROISyncStatus>,
    pub(super) refused: BTreeSet<Lease>,
    pub(super) acquire_gate: Option<Arc<Notify>>,
    pub(super) acquires: Vec<(String, ROISyncStatus, f64)>,
    pub(super) beats: Vec<(String, ROISyncStatus)>,
    pub(super) finishes: Vec<(String, ROISyncStatus, Option<ROIReport>)>,
}

#[derive(Clone, Default)]
pub(super) struct FakeCoordinator {
    state: Arc<Mutex<CoordinatorState>>,
}

impl FakeCoordinator {
    pub(super) fn sharing(&self, status: ROISyncStatus) {
        self.state.lock().unwrap().shared = Some(status);
    }

    pub(super) fn refusing(&self, lease: Lease) {
        self.state.lock().unwrap().refused.insert(lease);
    }

    pub(super) fn holding_acquires(&self, gate: Arc<Notify>) {
        self.state.lock().unwrap().acquire_gate = Some(gate);
    }

    pub(super) fn acquires(&self) -> Vec<(String, ROISyncStatus, f64)> {
        self.state.lock().unwrap().acquires.clone()
    }

    pub(super) fn beats(&self) -> Vec<(String, ROISyncStatus)> {
        self.state.lock().unwrap().beats.clone()
    }

    pub(super) fn finishes(&self) -> Vec<(String, ROISyncStatus, Option<ROIReport>)> {
        self.state.lock().unwrap().finishes.clone()
    }
}

pub(super) struct NoDeclaredCapabilities;

#[async_trait]
impl CapabilityLookup for NoDeclaredCapabilities {
    async fn supports(&self, _model: &str, _provider: Option<&str>, _key: &str) -> bool {
        false
    }
}

pub(super) struct ModelSupport;

#[async_trait]
impl CapabilityLookup for ModelSupport {
    async fn supports(&self, _model: &str, _provider: Option<&str>, _key: &str) -> bool {
        true
    }
}

impl CapabilityPredicates for ModelSupport {
    fn supports_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(true)
    }

    fn declared_value_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        ready(None)
    }

    fn is_explicitly_disabled_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn supports_native_streaming(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = Json> + Send {
        ready(Json::Null)
    }

    fn supports_response_schema(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }
}

#[async_trait]
impl SyncCoordinator for FakeCoordinator {
    async fn status(&self) -> Option<ROISyncStatus> {
        self.state.lock().unwrap().shared.clone()
    }

    async fn acquire(&self, owner: &str, status: &ROISyncStatus, scheduled_interval: f64) -> bool {
        let gate = {
            let mut state = self.state.lock().unwrap();
            state
                .acquires
                .push((owner.to_owned(), status.clone(), scheduled_interval));
            state.acquire_gate.clone()
        };
        if let Some(notify) = gate {
            notify.notified().await;
        }
        !self.state.lock().unwrap().refused.contains(&Lease::Acquire)
    }

    async fn heartbeat(&self, owner: &str, status: &ROISyncStatus) -> bool {
        let mut state = self.state.lock().unwrap();
        state.beats.push((owner.to_owned(), status.clone()));
        !state.refused.contains(&Lease::Heartbeat)
    }

    async fn finish(
        &self,
        owner: &str,
        status: &ROISyncStatus,
        report: Option<&ROIReport>,
    ) -> bool {
        let mut state = self.state.lock().unwrap();
        state
            .finishes
            .push((owner.to_owned(), status.clone(), report.cloned()));
        !state.refused.contains(&Lease::Finish)
    }
}

fn answer_body() -> String {
    let content = serde_json::to_string(ANSWER).unwrap();
    format!(r#"{{"choices":[{{"finish_reason":"stop","message":{{"content":{content}}}}}]}}"#)
}

fn answer_future(body: &str) -> CompletionFuture {
    let object: CompletionObject = RawValue::from_string(body.to_owned()).unwrap();
    Box::pin(ready(Ok(object)))
}

pub(super) fn answering_caller(count: Option<Arc<AtomicUsize>>) -> CompletionCaller {
    let body = answer_body();
    Arc::new(move |_request| {
        if let Some(counter) = count.as_ref() {
            counter.fetch_add(1, Ordering::SeqCst);
        }
        answer_future(&body)
    })
}

pub(super) fn recording_caller(sent: Arc<Mutex<Vec<ROICompletionRequest>>>) -> CompletionCaller {
    let answer = answering_caller(None);
    Arc::new(move |request| {
        sent.lock().unwrap().push(request.clone());
        answer(request)
    })
}

pub(super) fn failing_caller() -> CompletionCaller {
    Arc::new(|_request| {
        let future: CompletionFuture = Box::pin(ready(Err(SourceError::PaginationLimit)));
        future
    })
}

async fn wait_for_barrier(
    barrier: Arc<Barrier>,
    body: String,
) -> Result<CompletionObject, SourceError> {
    barrier.wait().await;
    let object: CompletionObject = RawValue::from_string(body).unwrap();
    Ok(object)
}

pub(super) fn barrier_caller(barrier: Arc<Barrier>) -> CompletionCaller {
    let body = answer_body();
    Arc::new(move |_request| {
        let future: CompletionFuture =
            Box::pin(wait_for_barrier(Arc::clone(&barrier), body.clone()));
        future
    })
}

pub(super) fn stalled_caller(gate: Arc<Notify>) -> CompletionCaller {
    Arc::new(move |_request| {
        let waiting = Arc::clone(&gate);
        let future: CompletionFuture = Box::pin(async move {
            waiting.notified().await;
            Err(SourceError::PaginationLimit)
        });
        future
    })
}

pub(super) struct Inputs {
    pub settings: ROISettings,
    pub repository: MemoryRepository,
    pub spend: FakeSpend,
    pub complete: CompletionCaller,
    pub routes: Routes,
    pub coordinator: Option<FakeCoordinator>,
    pub interval: f64,
}

impl Inputs {
    pub(super) fn run(&self) -> SyncRun<Routes, FakeSpend, MemoryRepository> {
        SyncRun {
            settings: self.settings.clone(),
            repository: self.repository.clone(),
            spend_reader: self.spend.clone(),
            complete: Arc::clone(&self.complete),
            github_transport: Some(self.routes.clone()),
            estimator_models: None,
            coordinator: self.lease(),
            scheduled_interval: self.interval,
        }
    }

    fn lease(&self) -> Option<Arc<dyn SyncCoordinator>> {
        let coordinator = self.coordinator.as_ref()?;
        let lease: Arc<dyn SyncCoordinator> = Arc::new(coordinator.clone());
        Some(lease)
    }
}

pub(super) fn inputs(repo_slug: &str, pull_numbers: &[i64]) -> Inputs {
    let routes = routes_for(repo_slug, pull_numbers);
    Inputs {
        settings: settings(&[repo_slug]),
        repository: MemoryRepository::default(),
        spend: FakeSpend::with_rows(vec![spend_row()]),
        complete: answering_caller(None),
        routes,
        coordinator: None,
        interval: 0.0_f64,
    }
}

fn fixed_clock() -> SyncClock {
    TestClock::at(FIXED_NOW).handle()
}

pub(super) fn manager(clock: Option<SyncClock>) -> SyncManager<Routes> {
    SyncManager::new(
        factory(),
        clock.unwrap_or_else(fixed_clock),
        Arc::new(Uuid::new_v4),
        Arc::new(NoDeclaredCapabilities),
    )
}

pub(super) fn system_manager() -> SyncManager<Routes> {
    SyncManager::new(
        factory(),
        Arc::new(Utc::now),
        Arc::new(Uuid::new_v4),
        Arc::new(NoDeclaredCapabilities),
    )
}

pub(super) fn no_capabilities() -> Arc<dyn CapabilityLookup> {
    Arc::new(NoDeclaredCapabilities)
}

pub(super) fn manager_from(factory: GitHubFactory<Routes>) -> SyncManager<Routes> {
    SyncManager::new(
        factory,
        fixed_clock(),
        Arc::new(Uuid::new_v4),
        Arc::new(NoDeclaredCapabilities),
    )
}

pub(super) fn manager_with_id(next_id: UuidSource) -> SyncManager<Routes> {
    SyncManager::new(
        factory(),
        fixed_clock(),
        next_id,
        Arc::new(NoDeclaredCapabilities),
    )
}

pub(super) fn manager_with_capabilities(
    capabilities: Arc<dyn CapabilityLookup>,
) -> SyncManager<Routes> {
    SyncManager::new(
        factory(),
        fixed_clock(),
        Arc::new(Uuid::new_v4),
        capabilities,
    )
}

pub(super) async fn drained() {
    for _ in 0_u32..16 {
        yield_now().await;
    }
}

pub(super) async fn settled(manager: &SyncManager<Routes>) {
    for _ in 0_u32..20_000 {
        if !manager.status().running {
            break;
        }
        yield_now().await;
    }
    for _ in 0_u32..64 {
        yield_now().await;
    }
}
