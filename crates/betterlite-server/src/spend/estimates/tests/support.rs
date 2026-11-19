use std::collections::BTreeMap;
use std::str::FromStr as _;
use std::sync::Arc;

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use betterlite_config::provider_budget_view::ProviderBudgetConfig;
use betterlite_core::cost::prepare::BillingTokenCounter;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_provider::pricing::grounding::ProviderFormulas;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_catalog_lookup::alias::{AliasMap, AliasTarget};
use betterlite_router::provider_budget_view::error::ProviderBudgetViewError;
use betterlite_router::provider_budget_view::view::ProviderBudgetReads;
use betterlite_tokenizer::count::error::CountError;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use tower::ServiceExt as _;

use crate::cost_estimate_route::testing::service_from;
use crate::cost_service::CostService;
use crate::key_fields::values::Amount;
use crate::spend::estimates::ports::{Clock, ConfiguredRouter, EstimateRouterPort};
use crate::spend::estimates::router::{EstimatesState, routes};

pub(crate) fn decimal(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap()
}

pub(crate) fn instant() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-03-01T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

// calculate_spend's token counting, answered with one fixed count so a price is arithmetic.
pub(crate) struct CountingCounter {
    pub(crate) messages: usize,
}

#[async_trait]
impl BillingTokenCounter for CountingCounter {
    async fn count_messages(&self, _model: &str, _messages: &[Json]) -> Result<usize, CountError> {
        Ok(self.messages)
    }

    async fn count_text(&self, _model: &str, _text: &str) -> Result<usize, CountError> {
        Ok(0)
    }
}

pub(crate) enum SpendAnswer {
    Value(Option<Decimal>),
    Fail,
}

pub(crate) struct FakeReads {
    pub(crate) spend: BTreeMap<String, SpendAnswer>,
    pub(crate) reset: BTreeMap<String, String>,
}

pub(crate) fn fake_reads(entries: &[(&str, Option<&str>)]) -> FakeReads {
    let spend = entries
        .iter()
        .map(|(provider, spend)| {
            let answer = spend.map_or(SpendAnswer::Value(None), |text| {
                SpendAnswer::Value(Some(decimal(text)))
            });
            ((*provider).to_owned(), answer)
        })
        .collect();
    FakeReads {
        spend,
        reset: BTreeMap::new(),
    }
}

#[async_trait]
impl ProviderBudgetReads for FakeReads {
    async fn current_provider_spend(
        &self,
        provider: &str,
    ) -> Result<Option<Decimal>, ProviderBudgetViewError> {
        match self.spend.get(provider) {
            Some(SpendAnswer::Value(spend)) => Ok(*spend),
            Some(SpendAnswer::Fail) => Err(ProviderBudgetViewError::Spend {
                key: format!("provider_spend:{provider}"),
            }),
            None => Ok(None),
        }
    }

    async fn provider_budget_reset_at(
        &self,
        provider: &str,
        _now: DateTime<Utc>,
    ) -> Result<Option<String>, ProviderBudgetViewError> {
        Ok(self.reset.get(provider).cloned())
    }
}

pub(crate) struct FixedClock(pub(crate) DateTime<Utc>);

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

pub(crate) fn deployment(name: &str, params: &str) -> DeploymentSpec {
    let fields: JsonObject = serde_json::from_str(params).unwrap();
    DeploymentSpec::new(
        name.to_owned(),
        fields,
        JsonObject::new(),
        JsonObject::new(),
        || "spec-id".to_owned(),
    )
    .unwrap()
}

pub(crate) fn alias(name: &str, target: &str) -> AliasMap {
    AliasMap::from([(name.to_owned(), AliasTarget::Name(target.to_owned()))])
}

pub(crate) fn provider_config(
    entries: &[(&str, Option<&str>, Option<&str>)],
) -> IndexMap<String, ProviderBudgetConfig> {
    entries
        .iter()
        .map(|(provider, limit, duration)| {
            let config = ProviderBudgetConfig {
                max_budget: limit.map(decimal),
                budget_duration: duration.map(str::to_owned),
            };
            ((*provider).to_owned(), config)
        })
        .collect()
}

pub(crate) fn configured_router(
    deployments: Vec<DeploymentSpec>,
    model_group_alias: AliasMap,
    provider_budget_config: Option<IndexMap<String, ProviderBudgetConfig>>,
    budget_limiter: Option<Arc<dyn ProviderBudgetReads>>,
) -> Arc<dyn EstimateRouterPort> {
    Arc::new(ConfiguredRouter {
        deployments,
        model_group_alias,
        provider_budget_config,
        budget_limiter,
    })
}

fn service() -> CostService<ProviderFormulas> {
    let asset = (BundledCatalog::default().asset)().unwrap();
    service_from(&asset).unwrap()
}

pub(crate) fn state(router: Option<Arc<dyn EstimateRouterPort>>) -> Arc<EstimatesState> {
    Arc::new(EstimatesState {
        cost: Arc::new(service()),
        counter: Arc::new(CountingCounter { messages: 10 }),
        router,
        clock: Arc::new(FixedClock(instant())),
    })
}

async fn answer(application: Router, request: Request<Body>) -> (StatusCode, String) {
    let response = application.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

pub(crate) async fn post_json(state: Arc<EstimatesState>, body: &str) -> (StatusCode, String) {
    let request = Request::post("/spend/calculate")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap();
    answer(routes(state), request).await
}

pub(crate) async fn get_budgets(state: Arc<EstimatesState>) -> (StatusCode, String) {
    let request = Request::get("/provider/budgets")
        .body(Body::empty())
        .unwrap();
    answer(routes(state), request).await
}

// The routes answer their declared wire models, so a test reads each answer back as a typed body.
#[derive(serde::Deserialize)]
pub(crate) struct SpendAnswerBody {
    #[serde(deserialize_with = "rust_decimal::serde::arbitrary_precision::deserialize")]
    pub(crate) cost: Decimal,
}

#[derive(serde::Deserialize)]
pub(crate) struct ErrorEnvelope {
    pub(crate) error: ErrorBody,
}

#[derive(serde::Deserialize)]
pub(crate) struct ErrorBody {
    pub(crate) message: String,
    #[serde(rename = "type")]
    pub(crate) kind: Json,
    pub(crate) param: Json,
    pub(crate) code: Json,
}

#[derive(serde::Deserialize)]
pub(crate) struct ValidationEnvelope {
    pub(crate) detail: Vec<ValidationDetail>,
}

#[derive(serde::Deserialize)]
pub(crate) struct ValidationDetail {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) loc: Vec<String>,
}

#[derive(serde::Deserialize)]
pub(crate) struct BudgetsEnvelope {
    pub(crate) providers: IndexMap<String, BudgetBody>,
}

#[derive(serde::Deserialize)]
pub(crate) struct BudgetBody {
    pub(crate) budget_limit: Option<Amount>,
    pub(crate) time_period: Option<String>,
    pub(crate) spend: Amount,
    pub(crate) budget_reset_at: Option<String>,
}

pub(crate) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(crate) fn cost_of(body: &str) -> Decimal {
    let answer: SpendAnswerBody = serde_json::from_str(body).unwrap();
    answer.cost
}

pub(crate) fn error_of(body: &str) -> ErrorBody {
    let envelope: ErrorEnvelope = serde_json::from_str(body).unwrap();
    envelope.error
}

pub(crate) fn validation_detail(body: &str) -> ValidationDetail {
    let envelope: ValidationEnvelope = serde_json::from_str(body).unwrap();
    envelope.detail.into_iter().next().unwrap()
}

pub(crate) fn budgets_of(body: &str) -> IndexMap<String, BudgetBody> {
    let envelope: BudgetsEnvelope = serde_json::from_str(body).unwrap();
    envelope.providers
}
