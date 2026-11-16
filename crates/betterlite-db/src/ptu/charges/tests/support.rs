use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;

use crate::budget_permission_rows::JsonData;
use crate::ptu::charges::decode::ModelInfo;
use crate::ptu::charges::pricing::PtuModel;

pub(crate) fn data(source: &str) -> JsonData {
    serde_json::from_str(source).unwrap()
}

pub(crate) fn info(source: &str) -> ModelInfo {
    serde_json::from_str(source).unwrap()
}

pub(crate) fn moment(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap()
        .with_timezone(&Utc)
}

pub(crate) fn day(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
}

pub(crate) fn ptu_model(id: &str, name: &str, team: &str, count: u32, rate: Decimal) -> PtuModel {
    PtuModel {
        model_id: id.to_owned(),
        model_name: name.to_owned(),
        team_id: team.to_owned(),
        ptu_count: count,
        cost_per_ptu_per_hour: rate,
        effective_from: moment("2026-01-01T00:00:00Z"),
        effective_to: None,
    }
}
