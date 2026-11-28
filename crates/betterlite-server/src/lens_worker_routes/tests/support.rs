use std::collections::VecDeque;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, HeaderValue, Request, StatusCode, header};
use betterlite_auth::reservation_cost::ModelInfoSource;
use betterlite_auth::reservation_cost::error::ModelInfoError;
use betterlite_auth::reservation_settlement::handle::ReservationHandle;
use betterlite_db::daily_spend_sql::bind::SqlBind;
use betterlite_db::lens_repository::database::{Database, ExecuteFuture, QueryFuture};
use betterlite_db::lens_repository::repository::LensRepository;
use betterlite_lens::models::execution::{Execution, ExecutionSource, Sample};
use betterlite_lens::models::job::{Job, JobStatus, Lens, Worker};
use betterlite_lens::models::settings::{Check, Scope};
use betterlite_lens::sources::parameters::Parameters;
use betterlite_lens::sources::storage::{
    ContentParameters, EvidenceParameters, SampleParameters, Storage,
};
use betterlite_logging::clickhouse::clickhouse_logging::context::LensAnalysis;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_redis::request_batch::lock;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::de::DeserializeOwned;
use tower::ServiceExt as _;

use crate::lens_access::state::LensRoutesState;
use crate::lens_access::tests::support::{
    FakeDatabase, FixedClock, Ids, Resolver, TestIssue, execution_id, job, lens_with, now, request,
    worker,
};
use crate::lens_billing::completion::{
    Billing, BillingError, ChatAnswer, ChatRunner, InternalAuthorizer, ReservationRelease,
};
use crate::lens_billing::internal_request::{ClientIp, Incoming, InternalRequest};
use crate::lens_inference::charge::{CostCalculator, CostError, CustomRates};
use crate::lens_inference::ports::{Clock, ModelCatalog};
use crate::lens_inference::quote::{Deployment, DeploymentParams};
use crate::lens_inference::request_body::AnalysisBody;
use crate::lens_worker_routes::model::Inference;
use crate::lens_worker_routes::{WorkerRoutesState, router};

pub(crate) const TOKEN: &str = "worker-token";
pub(crate) const TOKEN_DIGEST: &str =
    "dedd02022dba76562722a9463558e4124c82c5e6a09e925e6b11eec9a7042bf7";
pub(crate) const OTHER_TOKEN: &str = "other-token";

pub(crate) fn key_id() -> String {
    "a".repeat(64)
}

pub(crate) fn authorized(method: &str, uri: &str, body: Option<&str>) -> Request<Body> {
    with_token(method, uri, body, TOKEN)
}

pub(crate) fn with_token(
    method: &str,
    uri: &str,
    body: Option<&str>,
    token: &str,
) -> Request<Body> {
    let mut built = request(method, uri, None, body);
    built.headers_mut().insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}")).expect("a bearer header"),
    );
    built
}

pub(crate) fn with_header(
    mut built: Request<Body>,
    name: header::HeaderName,
    value: &str,
) -> Request<Body> {
    built
        .headers_mut()
        .insert(name, HeaderValue::from_str(value).expect("a header value"));
    built
}

pub(crate) fn base_lens(id: &str) -> Lens {
    let mut stored = lens_with(id, true);
    stored.budget_month = "2026-05".to_owned();
    stored
}

pub(crate) fn check(id: &str) -> Check {
    Check {
        id: id.to_owned(),
        instruction: "Look for checkout errors".to_owned(),
        enabled: true,
    }
}

pub(crate) fn with_checks(mut stored: Lens, checks: &[&str]) -> Lens {
    stored.settings.checks = checks.iter().map(|id| check(id)).collect();
    stored
}

pub(crate) fn running_job(id: &str, worker_id: &str, lease: DateTime<Utc>) -> Job {
    let mut stored = job(id, JobStatus::Running);
    stored.worker_id = Some(worker_id.to_owned());
    stored.lease_until = Some(lease);
    stored
}

pub(crate) fn worker_fixture() -> Worker {
    let mut stored = worker("w1");
    stored.analysis_key_id = Some(key_id());
    stored
}

pub(crate) fn worker_without_key() -> Worker {
    worker("w1")
}

pub(crate) fn team_worker() -> Worker {
    let mut stored = worker_fixture();
    stored.scope = Scope {
        team_id: "team-1".to_owned(),
        api_key_hash: String::new(),
        all_teams: false,
    };
    stored
}

pub(crate) fn trace_execution(index: usize) -> Execution {
    let trace_id = format!("t{index}");
    Execution {
        id: execution_id(&["traces", "team-1", trace_id.as_str(), ""]),
        source: ExecutionSource::Traces,
        trace_id: trace_id.clone(),
        trace_ref: String::new(),
        team_id: "team-1".to_owned(),
        name: trace_id,
        start_time: "2026-01-01T00:00:00Z".to_owned(),
        span_count: 1,
        root_seen: false,
        service: String::new(),
        metadata: Vec::new(),
    }
}

pub(crate) fn sample_of(executions: Vec<Execution>) -> Sample {
    let selected = i64::try_from(executions.len()).expect("a small sample");
    Sample {
        executions,
        eligible: selected,
        selected,
        next_offset: None,
        next_cursor: None,
    }
}

pub(crate) fn page(rows: usize, first: usize, eligible: i64, selected: i64) -> String {
    let entries: Vec<String> = (first..first.wrapping_add(rows))
        .map(|index| {
            format!(
                concat!(
                    r#"{{"selection_key":"k{index}","source":"traces","trace_id":"t{index}","#,
                    r#""team_id":"team-1","name":"t{index}","start_time":"2026-01-01T00:00:00Z","#,
                    r#""span_count":1,"root_seen":1,"eligible":{eligible},"selected":{selected}}}"#
                ),
                index = index,
                eligible = eligible,
                selected = selected
            )
        })
        .collect();
    format!("[{}]", entries.join(","))
}

pub(crate) fn progress_body(stage: &str, screened: i64) -> String {
    format!(r#"{{"stage":"{stage}","coverage":{{"screened":{screened}}}}}"#)
}

pub(crate) fn assessment(execution_id: &str, checks: &[&str]) -> String {
    let listed: Vec<String> = checks.iter().map(|id| format!("\"{id}\"")).collect();
    format!(
        r#"{{"execution_id":"{execution_id}","issue_checks":[{}]}}"#,
        listed.join(",")
    )
}

pub(crate) fn draft(check_id: &str, finding_id: Option<&str>, execution_id: &str) -> String {
    let existing = finding_id.map_or_else(String::new, |id| {
        format!(r#","existing_finding_id":"{id}""#)
    });
    format!(
        concat!(
            r#"{{"title":"Checkout retries","description":"The checkout retried twice","#,
            r#""check_id":"{check_id}","evidence":[{{"execution_id":"{execution_id}","#,
            r#""span_id":"s1","quote":"boom"}}]{existing}}}"#
        ),
        check_id = check_id,
        execution_id = execution_id,
        existing = existing
    )
}

pub(crate) fn result_body(assessments: &[String], findings: &[String], error: &str) -> String {
    format!(
        r#"{{"assessments":[{}],"findings":[{}],"coverage":{{"screened":5}},"error":"{error}"}}"#,
        assessments.join(","),
        findings.join(",")
    )
}

pub(crate) fn model_body(prompt: &str) -> String {
    format!(r#"{{"prompt":"{prompt}","purpose":"cluster"}}"#)
}

#[derive(Debug, thiserror::Error)]
#[error("the fake storage failed")]
pub(crate) struct TestError;

#[derive(Clone, Default)]
pub(crate) struct WorkerDatabase {
    pub(crate) inner: FakeDatabase,
    pub(crate) heartbeats: Arc<Mutex<Vec<String>>>,
    pub(crate) events: Arc<Mutex<Vec<&'static str>>>,
    pub(crate) reads: Arc<Mutex<u32>>,
    pub(crate) swap: Arc<Mutex<Option<(u32, Lens)>>>,
}

fn text_argument(args: &[SqlBind], index: usize) -> Option<&str> {
    match args.get(index) {
        Some(SqlBind::Text(Some(value))) => Some(value.as_str()),
        Some(_) | None => None,
    }
}

impl WorkerDatabase {
    pub(crate) fn seed_lens(&self, lens: Lens) {
        self.inner.seed_lens(lens);
    }

    pub(crate) fn seed_worker(&self, stored: Worker) {
        self.inner.seed_worker(stored.clone());
        self.inner
            .tokens
            .lock()
            .expect("the token lock")
            .push((TOKEN_DIGEST.to_owned(), stored));
    }

    pub(crate) fn lens(&self, id: &str) -> Lens {
        self.inner.lens(id)
    }

    pub(crate) fn worker(&self, id: &str) -> Worker {
        self.inner.worker(id)
    }

    pub(crate) fn heartbeats(&self) -> Vec<String> {
        self.heartbeats.lock().expect("the heartbeat lock").clone()
    }

    pub(crate) fn events(&self) -> Vec<&'static str> {
        self.events.lock().expect("the event lock").clone()
    }

    pub(crate) fn moved_on_read(&self, read: u32, replacement: Lens) {
        *self.swap.lock().expect("the swap lock") = Some((read, replacement));
    }

    fn record_heartbeat(&self, args: &[SqlBind]) {
        let (Some(stamp), Some(worker_id)) = (text_argument(args, 0), text_argument(args, 1))
        else {
            return;
        };
        self.heartbeats
            .lock()
            .expect("the heartbeat lock")
            .push(stamp.to_owned());
        let Ok(parsed) = DateTime::parse_from_rfc3339(stamp) else {
            return;
        };
        let mut workers = self.inner.workers.lock().expect("the worker lock");
        let slot = workers.iter_mut().find(|found| found.id == worker_id);
        if let Some(worker_record) = slot {
            worker_record.last_seen = parsed.with_timezone(&Utc);
        }
    }

    fn maybe_swap(&self) {
        let mut reads = self.reads.lock().expect("the read lock");
        *reads = reads.wrapping_add(1);
        let reached = *reads;
        drop(reads);
        let mut swap = self.swap.lock().expect("the swap lock");
        let Some((trigger, pending)) = swap.as_ref() else {
            return;
        };
        if *trigger != reached {
            return;
        }
        let replacement = pending.clone();
        *swap = None;
        drop(swap);
        let mut lenses = self.inner.lenses.lock().expect("the lens lock");
        let slot = lenses.iter_mut().find(|stored| stored.id == replacement.id);
        if let Some(stored) = slot {
            *stored = replacement;
        }
    }

    fn observe_query(&self, query: &str) {
        if query.contains("WITH previous") {
            self.events.lock().expect("the event lock").push("update");
        }
        if query.contains(r#""LiteLLM_Lens" WHERE id=$1"#) {
            self.maybe_swap();
        }
    }

    fn record_heartbeat_query(&self, query: &str, args: &[SqlBind]) -> bool {
        if !query.contains("'{last_seen}'") {
            return false;
        }
        self.events
            .lock()
            .expect("the event lock")
            .push("heartbeat");
        self.record_heartbeat(args);
        true
    }

    fn queried<'db>(&'db self, query: &'db str, args: &'db [SqlBind]) -> QueryFuture<'db> {
        self.observe_query(query);
        self.inner.query_raw(query, args)
    }

    fn written<'db>(&'db self, query: &'db str, args: &'db [SqlBind]) -> ExecuteFuture<'db> {
        if self.record_heartbeat_query(query, args) {
            return Box::pin(ready(Ok(1)));
        }
        self.inner.execute_raw(query, args)
    }
}

impl Database for WorkerDatabase {
    fn query_raw<'database>(
        &'database self,
        query: &'database str,
        args: &'database [SqlBind],
    ) -> QueryFuture<'database> {
        self.queried(query, args)
    }

    fn execute_raw<'database>(
        &'database self,
        query: &'database str,
        args: &'database [SqlBind],
    ) -> ExecuteFuture<'database> {
        self.written(query, args)
    }
}

enum Recorded {
    Sample(SampleParameters),
    Content(ContentParameters),
    Evidence(EvidenceParameters),
}

enum Reply {
    Sample,
    Content,
    Evidence,
}

#[derive(Clone, Default)]
pub(crate) struct WorkerStorage {
    pub(crate) samples: Arc<Mutex<Vec<SampleParameters>>>,
    pub(crate) pages: Arc<Mutex<VecDeque<String>>>,
    pub(crate) contents: Arc<Mutex<Vec<ContentParameters>>>,
    pub(crate) content_json: Arc<Mutex<String>>,
    pub(crate) evidence: Arc<Mutex<Vec<EvidenceParameters>>>,
    pub(crate) evidence_found: Arc<Mutex<bool>>,
    pub(crate) failure: Arc<Mutex<bool>>,
}

impl WorkerStorage {
    pub(crate) fn with_pages(pages: &[String]) -> Self {
        let storage = Self::default();
        *storage.evidence_found.lock().expect("the evidence lock") = true;
        let mut stored = storage.pages.lock().expect("the page lock");
        *stored = pages.iter().cloned().collect();
        drop(stored);
        storage
    }

    pub(crate) fn sample_calls(&self) -> Vec<SampleParameters> {
        self.samples.lock().expect("the sample lock").clone()
    }

    pub(crate) fn content_calls(&self) -> Vec<ContentParameters> {
        self.contents.lock().expect("the content lock").clone()
    }

    pub(crate) fn evidence_calls(&self) -> Vec<EvidenceParameters> {
        self.evidence.lock().expect("the evidence lock").clone()
    }

    pub(crate) fn answering_content(self, text: &str) -> Self {
        *self.content_json.lock().expect("the content json lock") = text.to_owned();
        self
    }

    pub(crate) fn answering_evidence(self, found: bool) -> Self {
        *self.evidence_found.lock().expect("the evidence lock") = found;
        self
    }

    pub(crate) fn failing(self) -> Self {
        *self.failure.lock().expect("the failure lock") = true;
        self
    }

    fn broken(&self) -> Result<(), TestError> {
        let failing = match self.failure.lock() {
            Ok(guard) => *guard,
            Err(_poisoned) => return Err(TestError),
        };
        if failing {
            return Err(TestError);
        }
        Ok(())
    }

    fn record(&self, recorded: Recorded) {
        match recorded {
            Recorded::Sample(parameters) => self
                .samples
                .lock()
                .expect("the sample lock")
                .push(parameters),
            Recorded::Content(parameters) => self
                .contents
                .lock()
                .expect("the content lock")
                .push(parameters),
            Recorded::Evidence(parameters) => self
                .evidence
                .lock()
                .expect("the evidence lock")
                .push(parameters),
        }
    }

    fn stored(&self, reply: &Reply) -> Vec<u8> {
        match reply {
            Reply::Sample => self
                .pages
                .lock()
                .expect("the page lock")
                .pop_front()
                .unwrap_or_else(|| "[]".to_owned())
                .into_bytes(),
            Reply::Content => {
                let text = self
                    .content_json
                    .lock()
                    .expect("the content json lock")
                    .clone();
                if text.is_empty() {
                    b"[]".to_vec()
                } else {
                    text.into_bytes()
                }
            }
            Reply::Evidence => {
                let found = *self.evidence_found.lock().expect("the evidence lock");
                if found {
                    br#"[{"count":1}]"#.to_vec()
                } else {
                    b"[]".to_vec()
                }
            }
        }
    }

    fn reply(&self, recorded: Recorded, reply: &Reply) -> Result<Vec<u8>, TestError> {
        self.record(recorded);
        self.broken()?;
        Ok(self.stored(reply))
    }
}

impl Storage for WorkerStorage {
    type Error = TestError;

    fn lens_availability(
        &self,
        _parameters: Parameters,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send {
        ready(Ok(b"[]".to_vec()))
    }

    fn lens_agents(
        &self,
        _parameters: Parameters,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send {
        ready(Ok(b"[]".to_vec()))
    }

    fn lens_sample(
        &self,
        parameters: SampleParameters,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send {
        ready(self.reply(Recorded::Sample(parameters), &Reply::Sample))
    }

    fn lens_content(
        &self,
        parameters: ContentParameters,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send {
        ready(self.reply(Recorded::Content(parameters), &Reply::Content))
    }

    fn lens_evidence(
        &self,
        parameters: EvidenceParameters,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send {
        ready(self.reply(Recorded::Evidence(parameters), &Reply::Evidence))
    }
}

pub(crate) struct FakeBilling {
    pub(crate) failed: bool,
    pub(crate) billed: Option<String>,
    pub(crate) calls: Mutex<Vec<String>>,
}

fn answer() -> ChatResponse {
    let json = concat!(
        r#"{"id":"x","object":"chat.completion","created":1,"model":"lens-model","#,
        r#""choices":[{"index":0,"message":{"role":"assistant","content":"{\"verdict\":\"ok\"}"}}],"#,
        r#""usage":{"prompt_tokens":2,"completion_tokens":4,"total_tokens":6}}"#
    );
    serde_json::from_str(json).expect("the fake completion decodes")
}

#[async_trait]
impl InternalAuthorizer for FakeBilling {
    async fn authorize(
        &self,
        _key_id: &str,
        request: &InternalRequest,
        _data: &AnalysisBody<'_>,
    ) -> Result<(), BillingError> {
        request.reservation.set(ReservationHandle::new(
            Decimal::ZERO,
            Vec::new(),
            Decimal::ZERO,
        ));
        Ok(())
    }
}

#[async_trait]
impl ChatRunner for FakeBilling {
    async fn run(
        &self,
        request: &InternalRequest,
        _data: &AnalysisBody<'_>,
    ) -> Result<ChatAnswer, BillingError> {
        let privacy = lock(&request.privacy.0).message_logging_disabled;
        self.calls.lock().expect("the call log").push(format!(
            "run scheme={} client={:?} privacy={privacy}",
            request.scheme, request.client
        ));
        if self.failed {
            return Err(BillingError::Http {
                status: 502,
                detail: "upstream failed".to_owned(),
            });
        }
        Ok(ChatAnswer {
            response: answer(),
            billed_cost: self.billed.clone(),
        })
    }
}

#[async_trait]
impl ReservationRelease for FakeBilling {
    async fn release(&self, _reservation: &ReservationHandle) {
        self.calls
            .lock()
            .expect("the call log")
            .push("release".to_owned());
    }
}

impl ClientIp for FakeBilling {
    fn get_mcp_client_ip(&self, incoming: &Incoming) -> Option<String> {
        self.calls.lock().expect("the call log").push(format!(
            "ip proto={:?} host={:?} scheme={}",
            incoming.x_forwarded_proto,
            incoming.host,
            incoming.scheme()
        ));
        incoming.x_forwarded_for.clone()
    }
}

pub(crate) struct DeploymentCatalog {
    pub(crate) configured: bool,
    pub(crate) deployments: Vec<Deployment>,
}

impl DeploymentCatalog {
    pub(crate) fn current() -> Self {
        Self {
            configured: true,
            deployments: vec![Deployment {
                litellm_params: DeploymentParams {
                    model: "lens-model".to_owned(),
                    input_cost_per_token: Some(Decimal::new(25, 2)),
                    output_cost_per_token: Some(Decimal::new(5, 1)),
                },
            }],
        }
    }

    pub(crate) fn missing() -> Self {
        Self {
            configured: false,
            deployments: Vec::new(),
        }
    }
}

impl ModelCatalog for DeploymentCatalog {
    fn configured(&self) -> bool {
        self.configured
    }

    fn deployments(&self, _model: &str, _team_id: Option<&str>) -> Vec<Deployment> {
        self.deployments.clone()
    }
}

#[derive(Debug)]
pub(crate) struct NoPrices;

impl ModelInfoSource for NoPrices {
    fn get_model_info(&self, _model: &str) -> Result<Option<JsonObject>, ModelInfoError> {
        Ok(None)
    }
}

pub(crate) struct Costs(pub(crate) Decimal);

impl CostCalculator for Costs {
    fn completion_cost(
        &self,
        _response: &ChatResponse,
        _custom: Option<&CustomRates<'_>>,
    ) -> Result<Decimal, CostError> {
        Ok(self.0)
    }
}

pub(crate) struct AnalysisClock {
    pub(crate) now: DateTime<Utc>,
}

impl Clock for AnalysisClock {
    fn now(&self) -> DateTime<Utc> {
        self.now
    }
}

fn inference(billing: &Arc<FakeBilling>, instant: DateTime<Utc>) -> Inference {
    let authorizer: Arc<dyn InternalAuthorizer> = Arc::<FakeBilling>::clone(billing);
    let runner: Arc<dyn ChatRunner> = Arc::<FakeBilling>::clone(billing);
    let releaser: Arc<dyn ReservationRelease> = Arc::<FakeBilling>::clone(billing);
    let client_ip: Arc<dyn ClientIp> = Arc::<FakeBilling>::clone(billing);
    let keys: Arc<Resolver> = Arc::new(Resolver::new());
    let models: Arc<dyn ModelCatalog> = Arc::new(DeploymentCatalog::current());
    let prices: Arc<dyn ModelInfoSource> = Arc::new(NoPrices);
    let costs: Arc<dyn CostCalculator> = Arc::new(Costs(Decimal::ZERO));
    let clock: Arc<dyn Clock> = Arc::new(AnalysisClock { now: instant });
    Inference {
        billing: Arc::new(Billing {
            authorizer,
            runner,
            releaser,
            client_ip,
        }),
        keys,
        models,
        prices,
        costs,
        clock,
        lens_analysis: LensAnalysis::default(),
    }
}

pub(crate) struct Harness {
    pub(crate) database: WorkerDatabase,
    pub(crate) storage: Option<WorkerStorage>,
    pub(crate) billing: Arc<FakeBilling>,
    pub(crate) inference: Inference,
    pub(crate) instant: DateTime<Utc>,
    pub(crate) ids: Arc<Ids>,
    pub(crate) connected: bool,
}

impl Harness {
    pub(crate) fn new() -> Self {
        Self::with_billing(false, Some("2.5"))
    }

    pub(crate) fn with_billing(failed: bool, billed: Option<&str>) -> Self {
        let instant = now();
        let billing = Arc::new(FakeBilling {
            failed,
            billed: billed.map(str::to_owned),
            calls: Mutex::new(Vec::new()),
        });
        Self {
            database: WorkerDatabase::default(),
            storage: Some(WorkerStorage::with_pages(&[])),
            inference: inference(&billing, instant),
            billing,
            instant,
            ids: Arc::new(Ids::default()),
            connected: true,
        }
    }

    pub(crate) fn without_storage(mut self) -> Self {
        self.storage = None;
        self
    }

    pub(crate) fn without_database(mut self) -> Self {
        self.connected = false;
        self
    }

    pub(crate) fn at(mut self, instant: DateTime<Utc>) -> Self {
        self.instant = instant;
        self.inference.clock = Arc::new(AnalysisClock { now: instant });
        self
    }

    pub(crate) fn without_router(&mut self) {
        let models: Arc<dyn ModelCatalog> = Arc::new(DeploymentCatalog::missing());
        self.inference.models = models;
    }

    pub(crate) fn state(&self) -> WorkerRoutesState<WorkerDatabase, WorkerStorage> {
        let repository = self.connected.then(|| {
            Arc::new(LensRepository {
                database: self.database.clone(),
            })
        });
        let keys: Arc<Resolver> = Arc::new(Resolver::new());
        WorkerRoutesState {
            routes: LensRoutesState {
                repository,
                storage: self.storage.clone(),
                models: None,
                keys,
                clock: Arc::new(FixedClock { now: self.instant }),
                ids: Arc::<Ids>::clone(&self.ids),
            },
            inference: self.inference.clone(),
        }
    }

    pub(crate) fn app(&self) -> Router {
        router(self.state())
    }

    pub(crate) fn calls(&self) -> Vec<String> {
        self.billing.calls.lock().expect("the call log").clone()
    }
}

pub(crate) async fn worker_call(app: Router, request: Request<Body>) -> WorkerReply {
    let response = app
        .oneshot(request)
        .await
        .expect("the worker router answers");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the reply body reads");
    WorkerReply {
        status,
        headers,
        body: bytes.to_vec(),
    }
}

pub(crate) struct WorkerReply {
    pub(crate) status: StatusCode,
    pub(crate) headers: HeaderMap,
    pub(crate) body: Vec<u8>,
}

impl WorkerReply {
    pub(crate) fn json<T>(&self) -> T
    where
        T: DeserializeOwned,
    {
        serde_json::from_slice(&self.body).expect("the reply body is json")
    }

    pub(crate) fn detail(&self) -> String {
        self.json::<DetailText>().detail
    }

    pub(crate) fn issues(&self) -> Vec<TestIssue> {
        self.json::<DetailIssues>().detail
    }

    pub(crate) fn header(&self, name: &str) -> Option<String> {
        self.headers
            .get(name)
            .map(|value| value.to_str().expect("an ascii header").to_owned())
    }
}

#[derive(serde::Deserialize)]
struct DetailText {
    detail: String,
}

#[derive(serde::Deserialize)]
struct DetailIssues {
    detail: Vec<TestIssue>,
}
