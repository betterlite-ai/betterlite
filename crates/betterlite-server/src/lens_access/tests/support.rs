use std::collections::BTreeMap;
use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use betterlite_auth::auth_identity::errors::AuthError;
use betterlite_auth::auth_identity::records::{Principal, SourceKey};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::daily_spend_sql::bind::SqlBind;
use betterlite_db::lens_repository::database::{Database, ExecuteFuture, QueryFuture, Row};
use betterlite_db::lens_repository::repository::LensRepository;
use betterlite_lens::models::finding::Finding;
use betterlite_lens::models::job::{Job, JobStatus, Lens, Worker};
use betterlite_lens::models::settings::LensSettings;
use betterlite_lens::sources::parameters::Parameters;
use betterlite_lens::sources::storage::{
    ContentParameters, EvidenceParameters, SampleParameters, Storage,
};
use chrono::{DateTime, TimeDelta, Utc};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tower::ServiceExt as _;

use crate::auth::auth_identity::AuthContext;
use crate::lens_access::state::{Clock, IdSource, LensRoutesState, ModelNames};
use crate::lens_admin_routes::router as admin_router;
use crate::lens_billing::key_validation::{KeyResolver, LensKey};

pub(crate) fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-05-01T12:00:00+00:00")
        .expect("a fixed instant")
        .with_timezone(&Utc)
}

pub(crate) fn admin() -> AuthContext {
    caller(Some("proxy_admin"), None, &[])
}

pub(crate) fn member() -> AuthContext {
    caller(Some("team"), Some("team-1"), &["other-model"])
}

pub(crate) fn caller(role: Option<&str>, team: Option<&str>, models: &[&str]) -> AuthContext {
    let key = auth_key(role, team, models);
    let mut principal: Principal = serde_json::from_str(
        r#"{"principal_type":"human","subject":"user-1","auth_method":"api_key"}"#,
    )
    .expect("the test principal decodes");
    principal.source_key = SourceKey(Some(key));
    AuthContext { principal }
}

pub(crate) fn auth_key(role: Option<&str>, team: Option<&str>, models: &[&str]) -> UserAPIKeyAuth {
    let mut fields = BTreeMap::new();
    if let Some(given_role) = role {
        fields.insert(
            "user_role".to_owned(),
            JsonData::String(given_role.to_owned()),
        );
    }
    if let Some(given_team) = team {
        fields.insert(
            "team_id".to_owned(),
            JsonData::String(given_team.to_owned()),
        );
    }
    if !models.is_empty() {
        let listed = models
            .iter()
            .map(|name| JsonData::String((*name).to_owned()))
            .collect();
        fields.insert("models".to_owned(), JsonData::Array(listed));
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("the test key constructs")
}

pub(crate) fn settings(model: &str) -> LensSettings {
    serde_json::from_str(&format!(
        r#"{{"name":"Nightly","model":"{model}","context":"Watch the checkout flow"}}"#
    ))
    .expect("the test settings decode")
}

pub(crate) fn settings_json(model: &str) -> String {
    format!(r#"{{"name":"Nightly","model":"{model}","context":"Watch the checkout flow"}}"#)
}

pub(crate) fn lens(id: &str) -> Lens {
    lens_with(id, true)
}

pub(crate) fn settings_with_ids(ids: &[&str]) -> LensSettings {
    let listed = ids
        .iter()
        .map(|id| format!("\"{id}\""))
        .collect::<Vec<_>>()
        .join(",");
    serde_json::from_str(&format!(
        r#"{{"name":"Nightly","model":"gpt-4o","context":"Watch the checkout flow","execution_ids":[{listed}]}}"#
    ))
    .expect("the selection settings decode")
}

pub(crate) fn lens_in_team(id: &str, team: &str) -> Lens {
    serde_json::from_str(&format!(
        r#"{{"id":"{id}","scope":{{"team_id":"{team}"}},"settings":{{"name":"Nightly","model":"gpt-4o","context":"Watch the checkout flow"}},"created_at":"2026-01-01T00:00:00Z","next_run_at":"2026-01-01T00:00:00Z","budget_month":"2026-01"}}"#
    ))
    .expect("the team lens decodes")
}

pub(crate) fn lens_with(id: &str, all_teams: bool) -> Lens {
    serde_json::from_str(&format!(
        r#"{{"id":"{id}","scope":{{"all_teams":{all_teams}}},"settings":{{"name":"Nightly","model":"gpt-4o","context":"Watch the checkout flow"}},"created_at":"2026-01-01T00:00:00Z","next_run_at":"2026-01-01T00:00:00Z","budget_month":"2026-01"}}"#
    ))
    .expect("the test lens decodes")
}

pub(crate) fn job(id: &str, status: JobStatus) -> Job {
    let mut job: Job = serde_json::from_str(&format!(
        r#"{{"id":"{id}","created_at":"2026-01-01T00:00:00Z","start":"2026-01-01T00:00:00Z","end":"2026-01-02T00:00:00Z","settings":{{"name":"Nightly","model":"gpt-4o","context":"Watch the checkout flow"}},"revision":1}}"#
    ))
    .expect("the test job decodes");
    job.status = status;
    job
}

pub(crate) fn worker(id: &str) -> Worker {
    serde_json::from_str(&format!(
        r#"{{"id":"{id}","name":"Lens worker","scope":{{"all_teams":true}},"last_seen":"1970-01-01T00:00:00Z"}}"#
    ))
    .expect("the test worker decodes")
}

pub(crate) fn finding(id: &str) -> Finding {
    serde_json::from_str(&format!(
        r#"{{"id":"{id}","title":"Title","description":"Ten characters","check_id":"c1","evidence":[{{"execution_id":"e1","span_id":"s1","quote":"q"}}],"status":"open","reason":"old","first_seen":"2026-01-01T00:00:00Z","last_seen":"2026-01-01T00:00:00Z","revision":1}}"#
    ))
    .expect("the test finding decodes")
}

pub(crate) struct FixedClock {
    pub(crate) now: DateTime<Utc>,
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.now
    }
}

#[derive(Default)]
pub(crate) struct Ids {
    counter: Mutex<u64>,
}

impl IdSource for Ids {
    fn new_id(&self) -> String {
        let mut counter = self.counter.lock().expect("the id counter lock");
        let next = counter.wrapping_add(1_u64);
        *counter = next;
        drop(counter);
        format!("id-{next}")
    }

    fn random_bytes(&self, count: usize) -> Vec<u8> {
        vec![0xab_u8; count]
    }
}

pub(crate) const TOKEN_BYTES: u8 = 0xab;

pub(crate) struct Catalog {
    pub(crate) names: Vec<String>,
    pub(crate) teams: Mutex<Vec<Option<String>>>,
}

impl ModelNames for Catalog {
    fn get_model_names(&self, team_id: Option<&str>) -> Vec<String> {
        self.teams
            .lock()
            .expect("the catalog team lock")
            .push(team_id.map(str::to_owned));
        self.names.clone()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    Active,
    Blocked,
    Session,
    Missing,
}

pub(crate) struct Resolver {
    pub(crate) answer: Mutex<Answer>,
    pub(crate) seen: Mutex<Vec<String>>,
    pub(crate) team: Option<String>,
}

impl Resolver {
    pub(crate) fn new() -> Self {
        Self {
            answer: Mutex::new(Answer::Active),
            seen: Mutex::new(Vec::new()),
            team: None,
        }
    }

    pub(crate) fn answering(mut self, answer: Answer) -> Self {
        *self.answer.get_mut().expect("the answer lock") = answer;
        self
    }

    pub(crate) fn seen(&self) -> Vec<String> {
        self.seen.lock().expect("the resolver seen lock").clone()
    }
}

#[async_trait]
impl KeyResolver for Resolver {
    async fn resolve(&self, hashed_token: &str) -> Result<LensKey, AuthError> {
        self.seen
            .lock()
            .expect("the resolver seen lock")
            .push(hashed_token.to_owned());
        let answer = *self.answer.lock().expect("the answer lock");
        match answer {
            Answer::Active => Ok(LensKey {
                blocked: false,
                is_session_token: false,
                team_id: self.team.clone(),
            }),
            Answer::Blocked => Ok(LensKey {
                blocked: true,
                is_session_token: false,
                team_id: None,
            }),
            Answer::Session => Ok(LensKey {
                blocked: false,
                is_session_token: true,
                team_id: None,
            }),
            Answer::Missing => Err(AuthError::KeyNotFound {
                hashed_token: hashed_token.to_owned(),
            }),
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct FakeStorage {
    pub(crate) samples: Arc<Mutex<Vec<SampleParameters>>>,
    pub(crate) contents: Arc<Mutex<Vec<ContentParameters>>>,
    pub(crate) sample_json: Arc<Mutex<String>>,
    pub(crate) content_json: Arc<Mutex<String>>,
    pub(crate) agents_json: Arc<Mutex<String>>,
    pub(crate) availability_json: Arc<Mutex<String>>,
}

fn stored(cell: &Mutex<String>) -> Vec<u8> {
    let text = cell.lock().expect("the storage fixture lock").clone();
    if text.is_empty() {
        b"[]".to_vec()
    } else {
        text.into_bytes()
    }
}

fn record<T>(cell: &Mutex<Vec<T>>, lock: &str, value: T) {
    cell.lock().expect(lock).push(value);
}

fn rows<T>(items: &Mutex<Vec<T>>, lock: &str) -> Vec<Row>
where
    T: Serialize,
{
    let guard = items.lock().expect(lock);
    guard.iter().map(encoded).collect()
}

impl FakeStorage {
    pub(crate) fn samples(&self) -> Vec<SampleParameters> {
        self.samples.lock().expect("the sample lock").clone()
    }

    pub(crate) fn contents(&self) -> Vec<ContentParameters> {
        self.contents.lock().expect("the content lock").clone()
    }

    pub(crate) fn returning_agents(self, text: &str) -> Self {
        *self.agents_json.lock().expect("the agents json lock") = text.to_owned();
        self
    }

    pub(crate) fn returning_availability(self, text: &str) -> Self {
        *self
            .availability_json
            .lock()
            .expect("the availability json lock") = text.to_owned();
        self
    }
}

impl Storage for FakeStorage {
    type Error = Infallible;

    fn lens_availability(
        &self,
        _parameters: Parameters,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send {
        ready(Ok(stored(&self.availability_json)))
    }

    fn lens_agents(
        &self,
        _parameters: Parameters,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send {
        ready(Ok(stored(&self.agents_json)))
    }

    fn lens_sample(
        &self,
        parameters: SampleParameters,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send {
        record(&self.samples, "the sample lock", parameters);
        ready(Ok(stored(&self.sample_json)))
    }

    fn lens_content(
        &self,
        parameters: ContentParameters,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send {
        record(&self.contents, "the content lock", parameters);
        ready(Ok(stored(&self.content_json)))
    }

    fn lens_evidence(
        &self,
        _parameters: EvidenceParameters,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send {
        ready(Ok(Vec::new()))
    }
}

#[derive(Clone, Default)]
pub(crate) struct FakeDatabase {
    pub(crate) lenses: Arc<Mutex<Vec<Lens>>>,
    pub(crate) workers: Arc<Mutex<Vec<Worker>>>,
    pub(crate) tokens: Arc<Mutex<Vec<(String, Worker)>>>,
    pub(crate) archived: Arc<Mutex<Vec<Job>>>,
    pub(crate) queries: Arc<Mutex<Vec<String>>>,
    pub(crate) job_offsets: Arc<Mutex<Vec<i64>>>,
    pub(crate) corrupt: Arc<Mutex<bool>>,
}

type QueryAnswer = fn(&FakeDatabase, &[SqlBind]) -> Vec<Row>;

// each fake query answers by the SQL fragment it carries, in the order the repository issues them
const QUERY_ANSWERS: &[(&str, QueryAnswer)] = &[
    ("WITH previous", FakeDatabase::updated),
    ("LIMIT 50 OFFSET", FakeDatabase::jobs),
    ("AS job", FakeDatabase::one_job),
    ("VALUES ($1,0,$2::jsonb)", FakeDatabase::created),
    ("'{analysis_key_id}'", FakeDatabase::billing),
    ("'{revoked}'", FakeDatabase::revoked),
    ("token_hash=$1", FakeDatabase::by_token),
    (
        "INSERT INTO \"LiteLLM_LensWorker\"",
        FakeDatabase::inserted_worker,
    ),
    (
        "UPDATE \"LiteLLM_LensWorker\"",
        FakeDatabase::replaced_worker,
    ),
    ("WHERE id=$1", FakeDatabase::one_lens),
];

impl FakeDatabase {
    pub(crate) fn job_offsets(&self) -> Vec<i64> {
        self.job_offsets.lock().expect("the offset lock").clone()
    }

    pub(crate) fn seed_lens(&self, lens: Lens) {
        self.lenses.lock().expect("the lens lock").push(lens);
    }

    pub(crate) fn seed_worker(&self, worker: Worker) {
        self.workers.lock().expect("the worker lock").push(worker);
    }

    pub(crate) fn seed_job(&self, job: Job) {
        self.archived.lock().expect("the job lock").push(job);
    }

    pub(crate) fn corrupted(&self) {
        *self.corrupt.lock().expect("the corrupt lock") = true;
    }

    pub(crate) fn lens(&self, id: &str) -> Lens {
        let lenses = self.lenses.lock().expect("the lens lock");
        lenses
            .iter()
            .find(|lens| lens.id == id)
            .expect("the stored lens")
            .clone()
    }

    pub(crate) fn worker(&self, id: &str) -> Worker {
        let workers = self.workers.lock().expect("the worker lock");
        workers
            .iter()
            .find(|worker| worker.id == id)
            .expect("the stored worker")
            .clone()
    }

    pub(crate) fn stored_token(&self) -> Option<String> {
        let tokens = self.tokens.lock().expect("the token lock");
        tokens.first().map(|(hash, _worker)| hash.clone())
    }

    pub(crate) fn queries(&self) -> Vec<String> {
        self.queries.lock().expect("the query lock").clone()
    }

    fn answer(&self, query: &str, args: &[SqlBind]) -> Vec<Row> {
        self.queries
            .lock()
            .expect("the query lock")
            .push(query.to_owned());
        if *self.corrupt.lock().expect("the corrupt lock") {
            return vec![Row {
                data: "not a record".to_owned(),
            }];
        }
        for &(needle, answer_query) in QUERY_ANSWERS {
            if query.contains(needle) {
                return answer_query(self, args);
            }
        }
        if query.contains("ORDER BY id") {
            return self.all_lenses();
        }
        if query.contains("\"LiteLLM_LensWorker\"") {
            return self.all_workers();
        }
        Vec::new()
    }

    fn all_lenses(&self) -> Vec<Row> {
        rows(&self.lenses, "the lens lock")
    }

    fn one_lens(&self, args: &[SqlBind]) -> Vec<Row> {
        let Some(id) = text_arg(args, 0) else {
            return Vec::new();
        };
        lens_rows(&self.lenses, id)
    }

    fn created(&self, args: &[SqlBind]) -> Vec<Row> {
        let Some(data) = text_arg(args, 1) else {
            return Vec::new();
        };
        let created: Lens = serde_json::from_str(data).expect("the created lens decodes");
        record(&self.lenses, "the lens lock", created);
        Vec::new()
    }

    fn updated(&self, args: &[SqlBind]) -> Vec<Row> {
        let Some(data) = text_arg(args, 0) else {
            return count(0);
        };
        let Some(id) = text_arg(args, 1) else {
            return count(0);
        };
        let Some(SqlBind::Integer(Some(version))) = args.get(2) else {
            return count(0);
        };
        let mut lenses = self.lenses.lock().expect("the lens lock");
        let Some(stored) = lenses.iter_mut().find(|lens| lens.id == id) else {
            return count(0);
        };
        if stored.version != *version {
            return count(0);
        }
        *stored = serde_json::from_str(data).expect("the updated lens decodes");
        drop(lenses);
        count(1)
    }

    fn jobs(&self, args: &[SqlBind]) -> Vec<Row> {
        let Some(id) = text_arg(args, 0) else {
            return Vec::new();
        };
        if let Some(SqlBind::Integer(Some(offset))) = args.get(1) {
            self.job_offsets
                .lock()
                .expect("the offset lock")
                .push(*offset);
        }
        let mut rows = Vec::new();
        let lenses = self.lenses.lock().expect("the lens lock");
        rows.extend(
            lenses
                .iter()
                .filter(|lens| lens.id == id)
                .flat_map(|lens| lens.jobs.iter())
                .map(encoded),
        );
        drop(lenses);
        rows.extend(
            self.archived
                .lock()
                .expect("the job lock")
                .iter()
                .map(encoded),
        );
        rows
    }

    fn one_job(&self, args: &[SqlBind]) -> Vec<Row> {
        let Some(lens_id) = text_arg(args, 0) else {
            return Vec::new();
        };
        let Some(job_id) = text_arg(args, 1) else {
            return Vec::new();
        };
        let lenses = self.lenses.lock().expect("the lens lock");
        let found = lenses
            .iter()
            .filter(|lens| lens.id == lens_id)
            .flat_map(|lens| lens.jobs.iter())
            .find(|job| job.id == job_id)
            .map(encoded);
        drop(lenses);
        if let Some(row) = found {
            return vec![row];
        }
        self.archived
            .lock()
            .expect("the job lock")
            .iter()
            .filter(|job| job.id == job_id)
            .map(encoded)
            .collect()
    }

    fn all_workers(&self) -> Vec<Row> {
        rows(&self.workers, "the worker lock")
    }

    fn by_token(&self, args: &[SqlBind]) -> Vec<Row> {
        let Some(hash) = text_arg(args, 0) else {
            return Vec::new();
        };
        self.tokens
            .lock()
            .expect("the token lock")
            .iter()
            .filter(|(stored, _worker)| stored == hash)
            .map(|(_hash, worker)| encoded(worker))
            .collect()
    }

    fn inserted_worker(&self, args: &[SqlBind]) -> Vec<Row> {
        let (Some(hash), Some(data)) = (text_arg(args, 1), text_arg(args, 2)) else {
            return Vec::new();
        };
        self.save_worker(hash, data);
        Vec::new()
    }

    fn save_worker(&self, hash: &str, data: &str) {
        let worker: Worker = serde_json::from_str(data).expect("the worker decodes");
        record(&self.workers, "the worker lock", worker.clone());
        record(&self.tokens, "the token lock", (hash.to_owned(), worker));
    }

    fn replaced_worker(&self, args: &[SqlBind]) -> Vec<Row> {
        let (Some(data), Some(id)) = (text_arg(args, 0), text_arg(args, 1)) else {
            return Vec::new();
        };
        replace_worker(&self.workers, id, data);
        Vec::new()
    }

    fn billing(&self, args: &[SqlBind]) -> Vec<Row> {
        let (Some(key_id), Some(id)) = (text_arg(args, 0), text_arg(args, 1)) else {
            return Vec::new();
        };
        apply_billing(&self.workers, id, key_id)
    }

    fn revoked(&self, args: &[SqlBind]) -> Vec<Row> {
        let Some(id) = text_arg(args, 0) else {
            return Vec::new();
        };
        retire_worker(&self.workers, id);
        Vec::new()
    }
}

fn lens_rows(lenses: &Mutex<Vec<Lens>>, id: &str) -> Vec<Row> {
    let guard = lenses.lock().expect("the lens lock");
    let matched = guard
        .iter()
        .filter(|lens| lens.id == id)
        .map(encoded)
        .collect();
    drop(guard);
    matched
}

fn replace_worker(workers: &Mutex<Vec<Worker>>, id: &str, data: &str) {
    let mut guard = workers.lock().expect("the worker lock");
    let Some(worker) = guard.iter_mut().find(|worker| worker.id == id) else {
        return;
    };
    *worker = serde_json::from_str(data).expect("the worker decodes");
    drop(guard);
}

fn apply_billing(workers: &Mutex<Vec<Worker>>, id: &str, key_id: &str) -> Vec<Row> {
    let mut guard = workers.lock().expect("the worker lock");
    let Some(worker) = guard.iter_mut().find(|worker| worker.id == id) else {
        return Vec::new();
    };
    if worker.revoked {
        return Vec::new();
    }
    worker.analysis_key_id = Some(key_id.to_owned());
    let rows = vec![encoded(worker)];
    drop(guard);
    rows
}

fn retire_worker(workers: &Mutex<Vec<Worker>>, id: &str) {
    let mut guard = workers.lock().expect("the worker lock");
    if let Some(worker) = guard.iter_mut().find(|worker| worker.id == id) {
        worker.revoked = true;
    }
    drop(guard);
}

fn text_arg(args: &[SqlBind], index: usize) -> Option<&str> {
    let Some(SqlBind::Text(Some(value))) = args.get(index) else {
        return None;
    };
    Some(value.as_str())
}

fn encoded<T>(value: &T) -> Row
where
    T: Serialize,
{
    Row {
        data: serde_json::to_string(value).expect("the stored record encodes"),
    }
}

fn count(rows: u64) -> Vec<Row> {
    vec![Row {
        data: rows.to_string(),
    }]
}

impl Database for FakeDatabase {
    fn query_raw<'database>(
        &'database self,
        query: &'database str,
        args: &'database [SqlBind],
    ) -> QueryFuture<'database> {
        Box::pin(ready(Ok(self.answer(query, args))))
    }

    fn execute_raw<'database>(
        &'database self,
        query: &'database str,
        args: &'database [SqlBind],
    ) -> ExecuteFuture<'database> {
        let rows = self.answer(query, args);
        Box::pin(ready(Ok(u64::from(!rows.is_empty()))))
    }
}

pub(crate) struct Harness {
    pub(crate) database: FakeDatabase,
    pub(crate) storage: Option<FakeStorage>,
    pub(crate) catalog: Option<Arc<Catalog>>,
    pub(crate) resolver: Arc<Resolver>,
    pub(crate) clock: FixedClock,
    pub(crate) ids: Arc<Ids>,
}

impl Harness {
    pub(crate) fn new() -> Self {
        Self {
            database: FakeDatabase::default(),
            storage: Some(FakeStorage::default()),
            catalog: Some(Arc::new(Catalog {
                names: vec!["gpt-4o".to_owned()],
                teams: Mutex::new(Vec::new()),
            })),
            resolver: Arc::new(Resolver::new()),
            clock: FixedClock { now: now() },
            ids: Arc::new(Ids::default()),
        }
    }

    pub(crate) fn without_storage(mut self) -> Self {
        self.storage = None;
        self
    }

    pub(crate) fn without_catalog(mut self) -> Self {
        self.catalog = None;
        self
    }

    pub(crate) fn state(&self) -> LensRoutesState<FakeDatabase, FakeStorage> {
        let models: Option<Arc<dyn ModelNames>> = self
            .catalog
            .as_ref()
            .map(|catalog| -> Arc<dyn ModelNames> { Arc::<Catalog>::clone(catalog) });
        LensRoutesState {
            repository: Some(Arc::new(LensRepository {
                database: self.database.clone(),
            })),
            storage: self.storage.clone(),
            models,
            keys: Arc::<Resolver>::clone(&self.resolver),
            clock: Arc::new(FixedClock {
                now: self.clock.now,
            }),
            ids: Arc::<Ids>::clone(&self.ids),
        }
    }

    pub(crate) fn state_without_database(&self) -> LensRoutesState<FakeDatabase, FakeStorage> {
        let mut state = self.state();
        state.repository = None;
        state
    }

    pub(crate) fn app(&self) -> Router {
        admin_router(self.state())
    }
}

pub(crate) fn repository(harness: &Harness) -> LensRepository<FakeDatabase> {
    LensRepository {
        database: harness.database.clone(),
    }
}

pub(crate) fn request(
    method: &str,
    uri: &str,
    caller: Option<&AuthContext>,
    body: Option<&str>,
) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    let built = builder
        .body(body.map_or_else(Body::empty, |text| Body::from(text.to_owned())))
        .expect("the test request builds");
    with_caller(built, caller)
}

fn with_caller(mut request: Request<Body>, caller: Option<&AuthContext>) -> Request<Body> {
    if let Some(context) = caller {
        request.extensions_mut().insert(context.clone());
    }
    request
}

pub(crate) async fn call(app: Router, request: Request<Body>) -> Reply {
    let response = app.oneshot(request).await.expect("the router answers");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the response body reads");
    Reply {
        status,
        body: bytes.to_vec(),
    }
}

pub(crate) struct Reply {
    pub(crate) status: StatusCode,
    pub(crate) body: Vec<u8>,
}

impl Reply {
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
}

#[derive(serde::Deserialize)]
struct DetailText {
    detail: String,
}

#[derive(serde::Deserialize)]
struct DetailIssues {
    detail: Vec<TestIssue>,
}

#[derive(serde::Deserialize)]
pub(crate) struct TestIssue {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) loc: Vec<String>,
}

pub(crate) fn execution_id(parts: &[&str]) -> String {
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE;
    URL_SAFE.encode(serde_json::to_vec(parts).expect("the execution parts encode"))
}

pub(crate) fn body_json<T>(value: &T) -> String
where
    T: Serialize,
{
    serde_json::to_string(value).expect("the test body encodes")
}

pub(crate) fn minutes_after(base: DateTime<Utc>, minutes: i64) -> DateTime<Utc> {
    base.checked_add_signed(TimeDelta::minutes(minutes))
        .expect("a representable instant")
}
