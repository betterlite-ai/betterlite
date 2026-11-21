use betterlite_models::savings_evidence::spend::BaselineCosts;
use rust_decimal::Decimal;
use serde::Serialize;

use super::model::BaselinePublication;
use crate::baseline::records::{BaselineAccountingRecord, DailyBaselineAttribution};

// _Change: one reversed-or-applied correction, carrying the identity its session and daily rows key on.
#[derive(Clone, Debug, Serialize)]
pub struct Change {
    pub request_id: String,
    pub publication: BaselinePublication,
    pub api_key: String,
    pub user_id: String,
    pub session_id: String,
    pub router_name: String,
    pub baseline_model: String,
    pub covered_delta: i64,
    pub actual_delta: Decimal,
    pub savings_delta: Decimal,
    pub daily: Option<DailyBaselineAttribution>,
}

// _change: new amounts minus the ones already published, relative to zero when either side is unknown.
#[must_use]
pub fn change(
    record: &BaselineAccountingRecord,
    old: Option<&BaselinePublication>,
    new: &BaselinePublication,
) -> Change {
    let previous = old.and_then(BaselinePublication::costs);
    let current = new.costs();
    Change {
        request_id: record.observation.request_id.clone(),
        publication: new.clone(),
        api_key: record.api_key.clone(),
        user_id: record
            .turn
            .as_ref()
            .map_or(String::new(), |turn| turn.user_id.clone()),
        session_id: record.session_id.clone(),
        router_name: record.router_name.clone(),
        baseline_model: record.baseline_model.clone(),
        covered_delta: i64::from(current.is_some()) - i64::from(previous.is_some()),
        actual_delta: delta(
            current.map(|value| value.actual),
            previous.map(|value| value.actual),
        ),
        savings_delta: delta(current.map(savings), previous.map(savings)),
        daily: record.daily.clone(),
    }
}

fn savings(value: BaselineCosts) -> Decimal {
    value
        .baseline
        .checked_sub(value.actual)
        .unwrap_or(Decimal::ZERO)
}

fn delta(current: Option<Decimal>, previous: Option<Decimal>) -> Decimal {
    let current = current.unwrap_or_default();
    let previous = previous.unwrap_or_default();
    current.checked_sub(previous).unwrap_or(Decimal::ZERO)
}
