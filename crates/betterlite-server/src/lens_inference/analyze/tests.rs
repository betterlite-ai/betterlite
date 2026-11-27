#![cfg(test)]

use std::collections::VecDeque;
use std::future::ready;
use std::str::FromStr as _;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_auth::auth_identity::errors::AuthError;
use betterlite_auth::reservation_cost::ModelInfoSource;
use betterlite_auth::reservation_cost::error::ModelInfoError;
use betterlite_auth::reservation_settlement::handle::ReservationHandle;
use betterlite_db::daily_spend_sql::bind::SqlBind;
use betterlite_db::lens_repository::database::{Database, ExecuteFuture, QueryFuture, Row};
use betterlite_db::lens_repository::repository::LensRepository;
use betterlite_lens::models::job::{Job, Lens, Worker};
use betterlite_lens::models::messages::{ModelRequest, ModelResult};
use betterlite_logging::clickhouse::clickhouse_logging::context::LensAnalysis;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_redis::request_batch::lock;
use chrono::{DateTime, Utc};
use rstest::rstest;
use rust_decimal::Decimal;

use super::{AnalysisInput, AnalyzeError, InferencePorts, analyze};
use crate::lens_billing::completion::{
    Billing, BillingError, ChatAnswer, ChatRunner, InternalAuthorizer, ReservationRelease,
};
use crate::lens_billing::internal_request::{ClientIp, Incoming, InternalRequest};
use crate::lens_billing::key_validation::{KeyResolver, LensKey};
use crate::lens_inference::charge::{CostCalculator, CostError, CustomRates};
use crate::lens_inference::ports::{Clock, ModelCatalog};
use crate::lens_inference::quote::{Deployment, DeploymentParams};
use crate::lens_inference::request_body::AnalysisBody;

const KEY: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const NOW: &str = "2026-02-01T00:00:00Z";

type Log = Arc<Mutex<Vec<String>>>;

#[derive(Clone)]
struct Store(Arc<Mutex<Option<Lens>>>);

struct FakeDatabase(Store);

impl Database for FakeDatabase {
    fn query_raw<'database>(
        &'database self,
        query: &'database str,
        args: &'database [SqlBind],
    ) -> QueryFuture<'database> {
        Box::pin(ready(Ok(rows(&self.0, query, args))))
    }

    fn execute_raw<'database>(
        &'database self,
        _query: &'database str,
        _args: &'database [SqlBind],
    ) -> ExecuteFuture<'database> {
        Box::pin(ready(Ok(0)))
    }
}

fn text_argument(args: &[SqlBind], index: usize) -> Option<&str> {
    match args.get(index) {
        Some(SqlBind::Text(Some(value))) => Some(value),
        _ => None,
    }
}

fn integer_argument(args: &[SqlBind], index: usize) -> Option<i64> {
    match args.get(index) {
        Some(SqlBind::Integer(Some(value))) => Some(*value),
        _ => None,
    }
}

fn rows(store: &Store, query: &str, args: &[SqlBind]) -> Vec<Row> {
    let mut guard = lock(&store.0);
    if query.starts_with("WITH previous") {
        let lens_id = text_argument(args, 1);
        let version = integer_argument(args, 2);
        let Some(previous) = guard.as_ref() else {
            return vec![Row {
                data: "0".to_owned(),
            }];
        };
        if Some(previous.id.as_str()) != lens_id || Some(previous.version) != version {
            return vec![Row {
                data: "0".to_owned(),
            }];
        }
        let Some(data) = text_argument(args, 0) else {
            return vec![Row {
                data: "0".to_owned(),
            }];
        };
        let updated: Lens = serde_json::from_str(data).unwrap();
        *guard = Some(updated);
        return vec![Row {
            data: "1".to_owned(),
        }];
    }
    match guard.as_ref() {
        Some(lens) if Some(lens.id.as_str()) == text_argument(args, 0) => vec![Row {
            data: serde_json::to_string(lens).unwrap(),
        }],
        _ => Vec::new(),
    }
}

struct BillingFake {
    log: Log,
    billed: Option<&'static str>,
    content: Option<&'static str>,
    choices: usize,
    run_failure: bool,
    lens_analysis: LensAnalysis,
}

#[async_trait]
impl InternalAuthorizer for BillingFake {
    async fn authorize(
        &self,
        _key_id: &str,
        request: &InternalRequest,
        _data: &AnalysisBody<'_>,
    ) -> Result<(), BillingError> {
        lock(&self.log).push("authorize".to_owned());
        request.reservation.set(ReservationHandle::new(
            Decimal::ZERO,
            Vec::new(),
            Decimal::ZERO,
        ));
        Ok(())
    }
}

#[async_trait]
impl ChatRunner for BillingFake {
    async fn run(
        &self,
        request: &InternalRequest,
        _data: &AnalysisBody<'_>,
    ) -> Result<ChatAnswer, BillingError> {
        let privacy = lock(&request.privacy.0).message_logging_disabled;
        let active = self.lens_analysis.is_lens_analysis();
        lock(&self.log).push(format!("run privacy={privacy} lens_analysis={active}"));
        if self.run_failure {
            return Err(BillingError::Http {
                status: 502,
                detail: "upstream failed".to_owned(),
            });
        }
        Ok(ChatAnswer {
            response: response(self.content, self.choices),
            billed_cost: self.billed.map(str::to_owned),
        })
    }
}

#[async_trait]
impl ReservationRelease for BillingFake {
    async fn release(&self, _reservation: &ReservationHandle) {
        lock(&self.log).push("release".to_owned());
    }
}

struct NoIp;

impl ClientIp for NoIp {
    fn get_mcp_client_ip(&self, _incoming: &Incoming) -> Option<String> {
        None
    }
}

struct KeyFake {
    blocked: bool,
}

#[async_trait]
impl KeyResolver for KeyFake {
    async fn resolve(&self, _hashed_token: &str) -> Result<LensKey, AuthError> {
        Ok(LensKey {
            blocked: self.blocked,
            is_session_token: false,
            team_id: Some("team-1".to_owned()),
        })
    }
}

struct CatalogFake {
    configured: bool,
    deployments: Vec<Deployment>,
}

impl ModelCatalog for CatalogFake {
    fn configured(&self) -> bool {
        self.configured
    }

    fn deployments(&self, _model: &str, _team_id: Option<&str>) -> Vec<Deployment> {
        self.deployments.clone()
    }
}

#[derive(Debug, Clone)]
enum Catalog {
    Known(Option<JsonObject>),
    Unavailable,
}

#[derive(Debug)]
struct PricesFake(Catalog);

impl ModelInfoSource for PricesFake {
    fn get_model_info(&self, _model: &str) -> Result<Option<JsonObject>, ModelInfoError> {
        match &self.0 {
            Catalog::Known(info) => Ok(info.clone()),
            Catalog::Unavailable => Err(ModelInfoError::Unavailable),
        }
    }
}

struct CalculatorFake {
    amount: &'static str,
}

impl CostCalculator for CalculatorFake {
    fn completion_cost(
        &self,
        _response: &ChatResponse,
        _custom: Option<&CustomRates<'_>>,
    ) -> Result<Decimal, CostError> {
        Ok(Decimal::from_str(self.amount).unwrap())
    }
}

struct ClockFake {
    readings: Mutex<VecDeque<DateTime<Utc>>>,
    last: DateTime<Utc>,
}

impl ClockFake {
    fn new(readings: &[DateTime<Utc>]) -> Self {
        let queue = VecDeque::from(readings.to_vec());
        let last = queue.back().copied().unwrap();
        Self {
            readings: Mutex::new(queue),
            last,
        }
    }
}

impl Clock for ClockFake {
    fn now(&self) -> DateTime<Utc> {
        lock(&self.readings).pop_front().unwrap_or(self.last)
    }
}

fn timestamp(text: &str) -> DateTime<Utc> {
    text.parse().unwrap()
}

fn deployment() -> Deployment {
    Deployment {
        litellm_params: DeploymentParams {
            model: "lens-model".to_owned(),
            input_cost_per_token: Some(Decimal::from_str("0.25").unwrap()),
            output_cost_per_token: Some(Decimal::from_str("0.5").unwrap()),
        },
    }
}

fn response(content: Option<&str>, choices: usize) -> ChatResponse {
    let message = content.map_or_else(
        || "{\"role\":\"assistant\",\"content\":null}".to_owned(),
        |text| format!("{{\"role\":\"assistant\",\"content\":\"{text}\"}}"),
    );
    let entries: Vec<String> = (0..choices)
        .map(|index| format!("{{\"index\":{index},\"message\":{message}}}"))
        .collect();
    let json = format!(
        concat!(
            "{{\"id\":\"x\",\"object\":\"chat.completion\",\"created\":1,\"model\":\"lens-model\",",
            "\"choices\":[{}],\"usage\":{{\"prompt_tokens\":2,",
            "\"completion_tokens\":4,\"total_tokens\":6}}}}"
        ),
        entries.join(",")
    );
    serde_json::from_str(&json).unwrap()
}

fn lens_fixture(
    job_status: &str,
    job_worker: &str,
    lease: &str,
    spent: &str,
    budget: &str,
) -> Lens {
    let json = [
        "{\"id\":\"lens-1\",\"scope\":{\"team_id\":\"team-1\"},",
        "\"settings\":{\"name\":\"n\",\"model\":\"lens-model\",\"monthly_budget\":",
        budget,
        "},\"created_at\":\"2026-01-10T08:00:00Z\",\"next_run_at\":\"2026-02-01T00:00:00Z\",",
        "\"budget_month\":\"2026-02\",\"spent\":",
        spent,
        ",\"jobs\":[{\"id\":\"job-1\",",
        "\"status\":\"",
        job_status,
        "\",\"created_at\":\"2026-01-10T08:00:00Z\",",
        "\"start\":\"2026-01-10T00:00:00Z\",\"end\":\"2026-01-10T08:00:00Z\",",
        "\"settings\":{\"name\":\"n\",\"model\":\"lens-model\"},\"revision\":1,",
        "\"worker_id\":\"",
        job_worker,
        "\",\"lease_until\":\"",
        lease,
        "\",\"cost\":0.5}]}",
    ]
    .concat();
    serde_json::from_str(&json).unwrap()
}

fn worker_fixture(key: Option<&str>) -> Worker {
    let key_field = key.map_or_else(String::new, |given| {
        format!(",\"analysis_key_id\":\"{given}\"")
    });
    let json = [
        "{\"id\":\"worker-1\",\"name\":\"w\",\"scope\":{\"team_id\":\"team-1\"},",
        "\"last_seen\":\"2026-01-10T08:00:00Z\"",
        key_field.as_str(),
        "}",
    ]
    .concat();
    serde_json::from_str(&json).unwrap()
}

fn body_fixture() -> ModelRequest {
    serde_json::from_str("{\"prompt\":\"hello\",\"purpose\":\"cluster\"}").unwrap()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Router {
    Configured,
    Missing,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum KeyState {
    Active,
    Blocked,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Run {
    Answers,
    Fails,
}

struct Setup {
    router: Router,
    key: KeyState,
    deployments: Vec<Deployment>,
    catalog: Catalog,
    calculator: &'static str,
    billed: Option<&'static str>,
    content: Option<&'static str>,
    choices: usize,
    run: Run,
    readings: Vec<DateTime<Utc>>,
    lens: Lens,
    job: Job,
    worker: Worker,
    body: ModelRequest,
}

fn setup() -> Setup {
    let lens = lens_fixture(
        "running",
        "worker-1",
        "2100-01-01T00:00:00Z",
        "1000.0",
        "10000.0",
    );
    let job = lens.jobs[0].clone();
    Setup {
        router: Router::Configured,
        key: KeyState::Active,
        deployments: vec![deployment()],
        catalog: Catalog::Known(None),
        calculator: "7.5",
        billed: Some("2.5"),
        content: Some("{}"),
        choices: 1,
        run: Run::Answers,
        readings: vec![timestamp(NOW), timestamp(NOW)],
        lens,
        job,
        worker: worker_fixture(Some(KEY)),
        body: body_fixture(),
    }
}

struct Facts {
    lens: Option<Lens>,
    log: Vec<String>,
    lens_analysis_active: bool,
    privacy_active: bool,
}

struct Dependencies {
    billing: Billing,
    log: Log,
    keys: KeyFake,
    models: CatalogFake,
    prices: PricesFake,
    costs: CalculatorFake,
    clock: ClockFake,
    store: Store,
    lens_analysis: LensAnalysis,
}

fn dependencies(setup: &Setup) -> Dependencies {
    let log: Log = Log::default();
    let lens_analysis = LensAnalysis::default();
    let fake = Arc::new(BillingFake {
        log: Arc::clone(&log),
        billed: setup.billed,
        content: setup.content,
        choices: setup.choices,
        run_failure: setup.run == Run::Fails,
        lens_analysis: lens_analysis.clone(),
    });
    // Arc::clone cannot widen to the fields' trait objects, so each is cloned as its own type
    let authorizer = Arc::clone(&fake);
    let runner = Arc::clone(&fake);
    let releaser = Arc::clone(&fake);
    let billing = Billing {
        authorizer,
        runner,
        releaser,
        client_ip: Arc::new(NoIp),
    };
    let keys = KeyFake {
        blocked: setup.key == KeyState::Blocked,
    };
    let models = CatalogFake {
        configured: setup.router == Router::Configured,
        deployments: setup.deployments.clone(),
    };
    let costs = CalculatorFake {
        amount: setup.calculator,
    };
    let clock = ClockFake::new(&setup.readings);
    let store = Store(Arc::new(Mutex::new(Some(setup.lens.clone()))));
    Dependencies {
        billing,
        log,
        keys,
        models,
        prices: PricesFake(setup.catalog.clone()),
        costs,
        clock,
        store,
        lens_analysis,
    }
}

async fn execute(setup: &Setup) -> (Result<ModelResult, AnalyzeError>, Facts) {
    let deps = dependencies(setup);
    let store = deps.store.clone();
    let log = Arc::clone(&deps.log);
    let repository = LensRepository {
        database: FakeDatabase(deps.store.clone()),
    };
    let incoming = Incoming::default();
    let ports = InferencePorts {
        billing: &deps.billing,
        keys: &deps.keys,
        models: &deps.models,
        prices: &deps.prices,
        costs: &deps.costs,
        clock: &deps.clock,
        lens_analysis: &deps.lens_analysis,
    };
    let input = AnalysisInput {
        lens: &setup.lens,
        job: &setup.job,
        worker: &setup.worker,
        body: &setup.body,
        incoming: &incoming,
    };
    let result = analyze(&repository, &ports, input).await;
    let facts = Facts {
        lens: lock(&store.0).clone(),
        log: lock(&log).clone(),
        lens_analysis_active: deps.lens_analysis.is_lens_analysis(),
        privacy_active: lock(&incoming.privacy.0).message_logging_disabled,
    };
    (result, facts)
}

fn amount(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap()
}

#[tokio::test]
async fn the_analysis_reserves_runs_and_settles() {
    let setup = setup();
    let (result, facts) = execute(&setup).await;
    let answer = result.unwrap();
    assert_eq!(answer.content, "{}");
    assert_eq!(answer.cost, amount("2.5"));
    let lens = facts.lens.unwrap();
    assert_eq!(lens.spent, amount("1002.5"));
    assert_eq!(lens.jobs[0].cost, amount("3.0"));
    assert_eq!(lens.version, 2);
    assert_eq!(
        facts.log,
        vec![
            "authorize".to_owned(),
            "run privacy=true lens_analysis=true".to_owned(),
            "release".to_owned(),
        ]
    );
    assert!(!facts.lens_analysis_active);
    assert!(!facts.privacy_active);
}

#[tokio::test]
async fn the_provider_cost_prices_an_unbilled_call() {
    let mut setup = setup();
    setup.billed = None;
    let (result, _facts) = execute(&setup).await;
    assert_eq!(result.unwrap().cost, amount("7.5"));
}

#[tokio::test]
async fn no_router_is_a_503_before_the_worker_key_is_read() {
    let mut setup = setup();
    setup.router = Router::Missing;
    let (result, _facts) = execute(&setup).await;
    let error = result.unwrap_err();
    assert_eq!(error.status(), 503);
    assert_eq!(error.detail(), "No analysis models are configured");
}

#[tokio::test]
async fn a_worker_without_an_analysis_key_is_a_409() {
    let mut setup = setup();
    setup.worker = worker_fixture(None);
    let (result, _facts) = execute(&setup).await;
    let error = result.unwrap_err();
    assert_eq!(error.status(), 409);
    assert_eq!(
        error.detail(),
        "Assign an analysis key to this worker in Lens setup"
    );
}

#[tokio::test]
async fn an_inactive_key_is_a_400() {
    let mut setup = setup();
    setup.key = KeyState::Blocked;
    let (result, _facts) = execute(&setup).await;
    let error = result.unwrap_err();
    assert_eq!(error.status(), 400);
    assert_eq!(
        error.detail(),
        "Choose an active virtual key for Lens analysis"
    );
}

#[tokio::test]
async fn a_model_the_router_no_longer_lists_is_a_400() {
    let mut setup = setup();
    setup.deployments.clear();
    let (result, _facts) = execute(&setup).await;
    let error = result.unwrap_err();
    assert_eq!(error.status(), 400);
    assert_eq!(error.detail(), "Analysis model is no longer available");
}

#[tokio::test]
async fn a_catalog_outage_is_a_500() {
    let mut setup = setup();
    setup.deployments = vec![Deployment {
        litellm_params: DeploymentParams {
            model: "lens-model".to_owned(),
            input_cost_per_token: None,
            output_cost_per_token: None,
        },
    }];
    setup.catalog = Catalog::Unavailable;
    let (result, _facts) = execute(&setup).await;
    assert_eq!(result.unwrap_err().status(), 500);
}

#[tokio::test]
async fn a_budget_with_no_room_is_a_402_and_writes_nothing() {
    let mut setup = setup();
    let over_budget = lens_fixture(
        "running",
        "worker-1",
        "2100-01-01T00:00:00Z",
        "9000.0",
        "10000.0",
    );
    setup.job = over_budget.jobs[0].clone();
    setup.lens = over_budget;
    let (result, facts) = execute(&setup).await;
    let error = result.unwrap_err();
    assert_eq!(error.status(), 402);
    assert_eq!(
        error.detail(),
        "Monthly lens budget reached; increase it or wait for next month"
    );
    let stored = facts.lens.unwrap();
    assert_eq!(stored.spent, amount("9000"));
    assert_eq!(stored.version, 0);
    assert_eq!(
        facts.log,
        vec!["authorize".to_owned(), "release".to_owned()]
    );
}

#[tokio::test]
async fn a_job_claimed_by_another_worker_is_a_409() {
    let mut setup = setup();
    let other_worker = lens_fixture(
        "running",
        "worker-2",
        "2100-01-01T00:00:00Z",
        "1000.0",
        "10000.0",
    );
    setup.job = other_worker.jobs[0].clone();
    setup.lens = other_worker;
    let (result, facts) = execute(&setup).await;
    let error = result.unwrap_err();
    assert_eq!(error.status(), 409);
    assert_eq!(error.detail(), "Job was cancelled or reassigned");
    assert_eq!(facts.lens.unwrap().version, 0);
}

#[tokio::test]
async fn an_expired_lease_is_a_409() {
    let mut setup = setup();
    let expired = lens_fixture("running", "worker-1", NOW, "1000.0", "10000.0");
    setup.job = expired.jobs[0].clone();
    setup.lens = expired;
    let (result, _facts) = execute(&setup).await;
    assert_eq!(result.unwrap_err().status(), 409);
}

#[tokio::test]
async fn a_call_the_model_answers_without_choices_fails_after_the_reserve() {
    let mut setup = setup();
    setup.choices = 0;
    let (result, facts) = execute(&setup).await;
    assert_eq!(result.unwrap_err().status(), 500);
    let stored = facts.lens.unwrap();
    assert_eq!(stored.spent, amount("5791.5"));
    assert_eq!(stored.jobs[0].cost, amount("4792"));
    assert_eq!(stored.version, 1);
    assert_eq!(facts.log.last().map(String::as_str), Some("release"));
    assert!(!facts.lens_analysis_active);
    assert!(!facts.privacy_active);
}

#[tokio::test]
async fn a_chat_failure_drops_both_scopes() {
    let mut setup = setup();
    setup.run = Run::Fails;
    let (result, facts) = execute(&setup).await;
    assert_eq!(result.unwrap_err().status(), 502);
    assert!(!facts.lens_analysis_active);
    assert!(!facts.privacy_active);
    assert_eq!(facts.log.last().map(String::as_str), Some("release"));
}

#[rstest]
#[case(None, "{}")]
#[case(Some(""), "{}")]
#[case(Some("answer"), "answer")]
#[tokio::test]
async fn the_answer_content_falls_back_to_an_empty_object(
    #[case] content: Option<&'static str>,
    #[case] expected: &'static str,
) {
    let mut setup = setup();
    setup.content = content;
    let (result, _facts) = execute(&setup).await;
    assert_eq!(result.unwrap().content, expected);
}
