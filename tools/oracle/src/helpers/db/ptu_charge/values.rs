use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::ptu::charges::deployment::RouterEntry;
use betterlite_db::ptu::charges::pricing::{PtuCharge, PtuModel};
use betterlite_oracle_helper_api::error::HelperError;
use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde_json::{Value, json};

use super::{EntryInput, ModelInput};

pub(super) fn parsed_models(inputs: &[ModelInput]) -> Result<Vec<PtuModel>, Box<HelperError>> {
    let mut parsed: Vec<PtuModel> = Vec::new();
    for input in inputs {
        parsed.push(ptu_model(input)?);
    }
    Ok(parsed)
}

pub(super) fn ptu_model(model: &ModelInput) -> Result<PtuModel, Box<HelperError>> {
    let rate = model
        .cost_per_ptu_per_hour
        .parse::<Decimal>()
        .map_err(|error| value_error(format!("cost_per_ptu_per_hour: {error}")))?;
    Ok(PtuModel {
        model_id: model.model_id.clone(),
        model_name: model.model_name.clone(),
        team_id: model.team_id.clone(),
        ptu_count: model.ptu_count,
        cost_per_ptu_per_hour: rate,
        effective_from: utc_instant(model.effective_from_micros)?,
        effective_to: model.effective_to_micros.map(utc_instant).transpose()?,
    })
}

pub(super) fn router_entry(entry: &EntryInput) -> RouterEntry {
    RouterEntry {
        model_name: entry.model_name.clone(),
        model_info: entry.model_info.clone().unwrap_or(JsonData::Null),
    }
}

pub(super) fn parsed_value(model: &PtuModel) -> Result<Value, Box<HelperError>> {
    let rate = float_value(model.cost_per_ptu_per_hour)?;
    Ok(json!({
        "model_id": model.model_id,
        "model_name": model.model_name,
        "team_id": model.team_id,
        "ptu_count": model.ptu_count,
        "cost_per_ptu_per_hour": rate,
        "effective_from_micros": model.effective_from.timestamp_micros(),
        "effective_to_micros": model.effective_to.map(|instant| instant.timestamp_micros()),
    }))
}

pub(super) fn charge_value(charge: &PtuCharge) -> Result<Value, Box<HelperError>> {
    let flat_cost = float_value(charge.flat_cost)?;
    Ok(json!({
        "team_id": charge.team_id,
        "model_id": charge.model_id,
        "model_name": charge.model_name,
        "flat_cost": flat_cost,
    }))
}

pub(super) fn utc_instant(micros: i64) -> Result<DateTime<Utc>, Box<HelperError>> {
    DateTime::<Utc>::from_timestamp_micros(micros)
        .ok_or_else(|| value_error(format!("instant {micros} is outside the supported range")))
}

pub(super) fn utc_day(text: &str) -> Result<NaiveDate, Box<HelperError>> {
    text.parse::<NaiveDate>()
        .map_err(|error| value_error(format!("day '{text}': {error}")))
}

pub(super) fn float_value(amount: Decimal) -> Result<f64, Box<HelperError>> {
    amount
        .to_f64()
        .ok_or_else(|| value_error(format!("amount {amount} does not fit an f64")))
}

pub(super) fn value_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
