use std::future::{Future, ready};

use betterlite_db::baseline::publication::change::change;
use betterlite_db::baseline::publication::decode::decode_baseline_publication;
use betterlite_db::baseline::publication::pricing::{PublicationPricing, baseline_publication};
use betterlite_db::baseline::publication::project::{StoredRecord, project_group};
use betterlite_db::baseline::records::canonical::serialize_record;
use betterlite_db::baseline::records::{
    BaselineEntity, DailyBaselineTarget, decode_baseline_record,
};
use betterlite_models::baseline_history::estimate::{BaselineEstimate, Provenance};
use betterlite_models::baseline_history::model::BaselineHistory;
use betterlite_models::baseline_history::usage::Usage;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::savings_pricing::context::{ModelIdentity, SavingsPricingContext};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use chrono::DateTime;
use rust_decimal::Decimal;

pub(crate) struct Op;

// The comparison pricing a case exercises: no deployment and no public price is ever looked up.
struct NoPricing;

static NO_PRICING: NoPricing = NoPricing;

impl PriceLookup for NoPricing {
    fn effective_model_info(&self, _model: &str, _provider: Option<&str>) -> Option<PriceRecord> {
        None
    }

    fn raw_cost_map_entry(&self, _key: &str) -> Option<PriceRecord> {
        None
    }

    fn effective_priced_model(&self, _model: &str, _provider: Option<&str>) -> Option<PricedModel> {
        None
    }
}

impl SavingsPricingContext for NoPricing {
    fn resolve_model(&self, _model: &str, _provider: Option<&str>) -> Option<ModelIdentity> {
        None
    }

    // The oracle has no deployment registry, so every case prices from supplied or public rates.
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

#[derive(serde::Deserialize)]
pub(crate) struct StoredInput {
    data: String,
    publication: Option<String>,
    conflicted: bool,
    started_at: f64,
}

#[derive(serde::Deserialize)]
struct EstimateInput {
    request_id: String,
    reason: String,
    provenance: Option<String>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(serde::Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Validate {
        record: String,
    },
    Canonical {
        record: String,
    },
    Publication {
        record: String,
        estimate: String,
        first_at: f64,
    },
    Change {
        record: String,
        old: Option<String>,
        new: String,
    },
    Project {
        history: String,
        stored: Vec<StoredInput>,
    },
    Adjustment {
        record: String,
        entity: BaselineEntity,
        entity_id: Option<String>,
        savings_delta: f64,
        request_id: String,
    },
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Validate { record } => validate(record),
            Input::Canonical { record } => canonical(record),
            Input::Publication {
                record,
                estimate,
                first_at,
            } => publication(record, estimate, *first_at),
            Input::Change { record, old, new } => changed(record, old.as_deref(), new),
            Input::Project { history, stored } => project(history, stored),
            Input::Adjustment {
                record,
                entity,
                entity_id,
                savings_delta,
                request_id,
            } => adjustment(
                record,
                *entity,
                entity_id.clone(),
                *savings_delta,
                request_id,
            ),
        }
    }
}

fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn validated_estimate(text: &str) -> Result<BaselineEstimate, Box<HelperError>> {
    let raw: EstimateInput =
        serde_json::from_str(text).map_err(|error| failure(error.to_string()))?;
    let provenance = match raw.provenance.as_deref() {
        None => None,
        Some("observed_identical") => Some(Provenance::ObservedIdentical),
        Some("modeled") => Some(Provenance::Modeled),
        Some(other) => return Err(failure(format!("unknown provenance {other}"))),
    };
    Ok(BaselineEstimate {
        request_id: raw.request_id,
        reason: raw.reason,
        provenance,
        usage: raw.usage,
    })
}

fn validate(text: &str) -> HelperResult {
    let _record =
        decode_baseline_record(text.as_bytes()).map_err(|error| failure(error.to_string()))?;
    serde_json::to_value(Accepted { accepted: true }).or_value_error()
}

#[derive(serde::Serialize)]
struct Accepted {
    accepted: bool,
}

fn canonical(text: &str) -> HelperResult {
    let record =
        decode_baseline_record(text.as_bytes()).map_err(|error| failure(error.to_string()))?;
    serde_json::to_value(serialize_record(&record)).or_value_error()
}

fn publication(text: &str, estimate: &str, first_at: f64) -> HelperResult {
    let pricing = PublicationPricing::new(&NO_PRICING, DateTime::UNIX_EPOCH);
    let record =
        decode_baseline_record(text.as_bytes()).map_err(|error| failure(error.to_string()))?;
    let decoded_estimate = validated_estimate(estimate)?;
    let value = baseline_publication(&pricing, &record, &decoded_estimate, first_at);
    serde_json::to_value(value).or_value_error()
}

fn changed(text: &str, old: Option<&str>, new: &str) -> HelperResult {
    let record =
        decode_baseline_record(text.as_bytes()).map_err(|error| failure(error.to_string()))?;
    let previous = old
        .map(|value| decode_baseline_publication(value.as_bytes()))
        .transpose()
        .map_err(|error| failure(error.to_string()))?;
    let current =
        decode_baseline_publication(new.as_bytes()).map_err(|error| failure(error.to_string()))?;
    let value = change(&record, previous.as_ref(), &current);
    serde_json::to_value(value).or_value_error()
}

fn project(history: &str, stored: &[StoredInput]) -> HelperResult {
    let decoded: BaselineHistory = serde_json::from_str(history)?;
    let rows: Vec<StoredRecord> = stored
        .iter()
        .map(|row| StoredRecord {
            data: row.data.clone(),
            publication: row.publication.clone(),
            conflicted: row.conflicted,
            started_at: row.started_at,
        })
        .collect();
    let pricing = PublicationPricing::new(&NO_PRICING, DateTime::UNIX_EPOCH);
    let (_, changes) = project_group(&pricing, &(decoded, Vec::new()), &rows).or_value_error()?;
    serde_json::to_value(changes).or_value_error()
}

fn adjustment(
    text: &str,
    entity: BaselineEntity,
    entity_id: Option<String>,
    savings_delta: f64,
    request_id: &str,
) -> HelperResult {
    let record =
        decode_baseline_record(text.as_bytes()).map_err(|error| failure(error.to_string()))?;
    let daily = record
        .daily
        .as_ref()
        .ok_or_else(|| failure("record has no daily attribution".to_owned()))?;
    let target = DailyBaselineTarget { entity, entity_id };
    let delta = Decimal::try_from(savings_delta).map_err(|error| failure(error.to_string()))?;
    let row = daily.adjustment(&target, delta, request_id);
    serde_json::to_value(row).or_value_error()
}
