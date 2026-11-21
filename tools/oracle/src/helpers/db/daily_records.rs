use std::collections::BTreeMap;
use std::fmt::Display;
use std::future::{Future, ready};

use betterlite_db::daily_records::baseline_targets::baseline_daily_attribution;
use betterlite_db::daily_records::metadata::request_status;
use betterlite_db::daily_records::projection::rows::base_spend_row;
use betterlite_db::daily_records::projection::{
    CallTypeEndpoints, DailyPayload, DailyPricing, common_daily_transaction,
};
use betterlite_db::daily_spend_sql::entities::DailySpendEntity;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::savings_pricing::context::{ModelIdentity, SavingsPricingContext};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use chrono::DateTime;
use jiff::tz::TimeZoneDatabase;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    operation: Operation,
    payload: String,
    entity: Option<DailySpendEntity>,
    #[serde(default)]
    prices: BTreeMap<String, PriceRecord>,
    #[serde(default)]
    endpoints: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Status,
    Common,
    Baseline,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(failure)?;
        runtime.block_on(answer(input))
    }
}

// The internal rates come from the case rather than a catalog, so both sides price the same evidence.
async fn answer(input: &Input) -> HelperResult {
    let payload = serde_json::from_str::<DailyPayload>(&input.payload).map_err(failure)?;
    let prices = CasePrices {
        prices: input.prices.clone(),
    };
    let endpoints = CaseEndpoints {
        routes: input.endpoints.clone(),
    };
    let zone = TimeZoneDatabase::none();
    let pricing = DailyPricing {
        context: &prices,
        zone: &zone,
        fallback_moment: DateTime::UNIX_EPOCH,
    };
    match input.operation {
        Operation::Status => Ok(status_answer(&payload)),
        Operation::Common => common_answer(&payload, input.entity, &pricing, &endpoints).await,
        Operation::Baseline => Ok(baseline_answer(&payload, &pricing, &endpoints).await),
    }
}

fn status_answer(payload: &DailyPayload) -> Value {
    json!({
        "status": request_status(payload.metadata.as_deref().map(str::as_bytes)),
        "raised": false,
    })
}

async fn common_answer(
    payload: &DailyPayload,
    entity: Option<DailySpendEntity>,
    pricing: &DailyPricing<'_, CasePrices>,
    endpoints: &CaseEndpoints,
) -> HelperResult {
    let Some(entity) = entity else {
        return Err(failure("a common operation needs an entity"));
    };
    let (projection, raised) =
        outcome(common_daily_transaction(payload, entity, pricing, endpoints).await);
    let base = projection.map(|record| base_spend_row(&record.base));
    Ok(json!({"base": base, "raised": raised}))
}

async fn baseline_answer(
    payload: &DailyPayload,
    pricing: &DailyPricing<'_, CasePrices>,
    endpoints: &CaseEndpoints,
) -> Value {
    let (attribution, raised) =
        outcome(baseline_daily_attribution(payload, pricing, endpoints).await);
    json!({"attribution": attribution, "raised": raised})
}

// Both operations answer the Python side the same way: the value, or a null with the raise flag the case compares.
fn outcome<Value, Error>(result: Result<Option<Value>, Error>) -> (Option<Value>, bool) {
    match result {
        Ok(Some(value)) => (Some(value), false),
        Ok(None) => (None, false),
        Err(_) => (None, true),
    }
}

// The case's catalog: one exact "provider/model" record, with no deployment registry behind it.
struct CasePrices {
    prices: BTreeMap<String, PriceRecord>,
}

impl PriceLookup for CasePrices {
    fn effective_model_info(&self, model: &str, provider: Option<&str>) -> Option<PriceRecord> {
        self.prices.get(&price_key(model, provider)).cloned()
    }

    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord> {
        self.prices.get(key).cloned()
    }

    fn effective_priced_model(&self, model: &str, provider: Option<&str>) -> Option<PricedModel> {
        let record = self.effective_model_info(model, provider)?;
        Some(PricedModel {
            record,
            tiered_pricing: Vec::new(),
            off_peak_pricing: None,
            off_peak_cache_read_present: false,
        })
    }
}

impl SavingsPricingContext for CasePrices {
    fn resolve_model(&self, model: &str, provider: Option<&str>) -> Option<ModelIdentity> {
        Some(ModelIdentity {
            model: model.to_owned(),
            provider: provider.unwrap_or_default().to_owned(),
        })
    }

    fn deployment_price(
        &self,
        _deployment_id: Option<&str>,
        _model: &str,
    ) -> impl Future<Output = Option<PricedModel>> + Send {
        ready(None)
    }

    fn public_lookup(&self) -> &dyn PriceLookup {
        self
    }
}

fn price_key(model: &str, custom_llm_provider: Option<&str>) -> String {
    match custom_llm_provider.filter(|provider| !provider.is_empty()) {
        Some(provider) => format!("{provider}/{model}"),
        None => model.to_owned(),
    }
}

// ROUTE_ENDPOINT_MAPPING as the case supplies it, since the map B004 owns is read-only here.
struct CaseEndpoints {
    routes: BTreeMap<String, String>,
}

impl CallTypeEndpoints for CaseEndpoints {
    fn endpoint(&self, call_type: &str) -> Option<&str> {
        self.routes.get(call_type).map(String::as_str)
    }
}

fn failure<Error>(error: Error) -> Box<HelperError>
where
    Error: Display,
{
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
