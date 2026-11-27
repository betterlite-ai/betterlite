use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use betterlite_auth::reservation_cost::ModelInfoSource;
use betterlite_auth::reservation_cost::error::ModelInfoError;
use betterlite_db::lens_repository::repository::LensRepository;
use betterlite_lens::models::job::{Job, Lens};
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_redis::request_batch::lock;
use betterlite_server::lens_inference::charge::{self, CostCalculator, CostError, CustomRates};
use betterlite_server::lens_inference::quote::{self, Deployment};
use betterlite_server::lens_inference::request_body::{AnalysisBody, RequestIds};
use betterlite_server::lens_inference::reservation::{self, ReserveRequest};
use betterlite_server::lens_inference::settlement::{self, SettleRequest};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tokio::runtime::{Builder, Runtime};

mod store;

use self::store::{FakeDatabase, Store};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
enum Input {
    Quote(QuoteInput),
    Reserve(AnalyzeInput),
    Settle(AnalyzeInput),
    Charge(ChargeInput),
    RequestBody(AnalyzeInput),
}

#[derive(Deserialize)]
struct QuoteInput {
    prompt: String,
    deployments: Vec<Deployment>,
    #[serde(default)]
    model_info: BTreeMap<String, JsonObject>,
}

#[derive(Deserialize)]
struct AnalyzeInput {
    lens: Lens,
    job_id: String,
    worker_id: String,
    prompt: String,
    deployments: Vec<Deployment>,
    #[serde(default)]
    model_info: BTreeMap<String, JsonObject>,
    now: Vec<DateTime<Utc>>,
    #[serde(default)]
    team_id: Option<String>,
    #[serde(default, with = "rust_decimal::serde::float_option")]
    cost: Option<Decimal>,
    #[serde(default)]
    job: Option<Job>,
}

#[derive(Deserialize)]
struct ChargeInput {
    deployments: Vec<Deployment>,
    response: ChatResponse,
    #[serde(with = "rust_decimal::serde::float")]
    estimate: Decimal,
}

#[derive(Serialize)]
struct FloatAmount(#[serde(with = "rust_decimal::serde::float")] Decimal);

#[derive(Serialize)]
struct LensState {
    version: i64,
    spent: FloatAmount,
    budget_month: String,
    cost: FloatAmount,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Quote(call) => estimate_value(call),
            Input::Charge(call) => charge_value(call),
            Input::Reserve(call) => update_value(call, Update::Reserve),
            Input::Settle(call) => update_value(call, Update::Settle),
            Input::RequestBody(call) => body_value(call),
        }
    }
}

#[derive(Debug)]
struct Catalog(BTreeMap<String, JsonObject>);

impl ModelInfoSource for Catalog {
    fn get_model_info(&self, model: &str) -> Result<Option<JsonObject>, ModelInfoError> {
        Ok(self.0.get(model).cloned())
    }
}

// B068's completion_cost for the custom rates a single deployment carries; the oracle only prices those
struct CustomCosts;

impl CostCalculator for CustomCosts {
    fn completion_cost(
        &self,
        response: &ChatResponse,
        custom: Option<&CustomRates<'_>>,
    ) -> Result<Decimal, CostError> {
        let Some(custom) = custom else {
            return Err(CostError::Unavailable);
        };
        let prompt = Decimal::from(response.usage.prompt_tokens)
            .checked_mul(custom.input_cost_per_token)
            .ok_or(CostError::Unavailable)?;
        let completion = Decimal::from(response.usage.completion_tokens)
            .checked_mul(custom.output_cost_per_token)
            .ok_or(CostError::Unavailable)?;
        prompt.checked_add(completion).ok_or(CostError::Unavailable)
    }
}

fn estimate(call: &AnalyzeInput) -> Result<Decimal, Box<HelperError>> {
    let catalog = Catalog(call.model_info.clone());
    quote::quote(&call.deployments, &call.prompt, &catalog)
        .map_err(|error| http_error(500, error.to_string()))
}

fn estimate_value(call: &QuoteInput) -> HelperResult {
    let catalog = Catalog(call.model_info.clone());
    let estimate = quote::quote(&call.deployments, &call.prompt, &catalog)
        .map_err(|error| http_error(500, error.to_string()))?;
    serde_json::to_value(FloatAmount(estimate)).map_err(|error| invalid(&error.to_string()))
}

fn charge_value(call: &ChargeInput) -> HelperResult {
    let cost = charge::completion_charge(
        &call.deployments,
        &call.response,
        call.estimate,
        &CustomCosts,
    )
    .map_err(|error| http_error(500, error.to_string()))?;
    serde_json::to_value(FloatAmount(cost)).map_err(|error| invalid(&error.to_string()))
}

fn lens_state(lens: &Lens) -> LensState {
    LensState {
        version: lens.version,
        spent: FloatAmount(lens.spent),
        budget_month: lens.budget_month.clone(),
        cost: FloatAmount(lens.jobs.first().map_or(Decimal::ZERO, |job| job.cost)),
    }
}

struct Prepared {
    store: Store,
    repository: LensRepository<FakeDatabase>,
    runtime: Runtime,
    estimate: Decimal,
}

fn prepare(call: &AnalyzeInput) -> Result<Prepared, Box<HelperError>> {
    let estimate = estimate(call)?;
    let store = Store(Arc::new(Mutex::new(Some(call.lens.clone()))));
    let repository = LensRepository {
        database: FakeDatabase(store.clone()),
    };
    let runtime = Builder::new_current_thread()
        .build()
        .map_err(|error| invalid(&error.to_string()))?;
    Ok(Prepared {
        store,
        repository,
        runtime,
        estimate,
    })
}

fn updated_state(prepared: &Prepared) -> HelperResult {
    let lens = lock(&prepared.store.0)
        .clone()
        .ok_or_else(|| invalid("the lens store lost its lens"))?;
    serde_json::to_value(lens_state(&lens)).map_err(|error| invalid(&error.to_string()))
}

#[derive(Clone, Copy)]
enum Update {
    Reserve,
    Settle,
}

fn reserve_call(call: &AnalyzeInput, prepared: &Prepared) -> Result<(), Box<HelperError>> {
    let (month_now, lease_now) = match call.now.as_slice() {
        [month, lease] => (*month, *lease),
        _ => return Err(invalid("reserve needs two clock readings")),
    };
    let request = ReserveRequest {
        lens_id: &call.lens.id,
        job_id: &call.job_id,
        worker_id: &call.worker_id,
        month_now,
        lease_now,
        estimate: prepared.estimate,
    };
    prepared
        .runtime
        .block_on(reservation::reserve(&prepared.repository, &request))
        .map_err(|error| http_error(error.status(), error.to_string()))
}

fn settle_call(call: &AnalyzeInput, prepared: &Prepared) -> Result<(), Box<HelperError>> {
    let Some(cost) = call.cost else {
        return Err(invalid("settle needs the billed cost"));
    };
    let Some(month_now) = call.now.first().copied() else {
        return Err(invalid("settle needs one clock reading"));
    };
    let request = SettleRequest {
        lens_id: &call.lens.id,
        job_id: &call.job_id,
        month_now,
        estimate: prepared.estimate,
        cost,
    };
    prepared
        .runtime
        .block_on(settlement::settle(&prepared.repository, &request))
        .map_err(|error| http_error(500, error.to_string()))
}

fn update_value(call: &AnalyzeInput, update: Update) -> HelperResult {
    let prepared = prepare(call)?;
    match update {
        Update::Reserve => reserve_call(call, &prepared)?,
        Update::Settle => settle_call(call, &prepared)?,
    }
    updated_state(&prepared)
}

fn job_of(call: &AnalyzeInput) -> Option<&Job> {
    match &call.job {
        Some(job) => Some(job),
        None => call.lens.jobs.iter().find(|job| job.id == call.job_id),
    }
}

fn body_value(call: &AnalyzeInput) -> HelperResult {
    let Some(job) = job_of(call) else {
        return Err(invalid("the lens has no such job"));
    };
    let body = AnalysisBody::new(
        &job.settings.model,
        &call.prompt,
        RequestIds {
            lens_id: &call.lens.id,
            run_id: &call.job_id,
            worker_id: &call.worker_id,
            team_id: call.team_id.as_deref(),
        },
    );
    Ok(serde_json::to_value(body)?)
}

fn invalid(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

fn http_error(status: u16, detail: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "HTTPException".to_owned(),
        status: Some(status),
        error_type: None,
        param: None,
        code: None,
        message: detail,
    })
}
