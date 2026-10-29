use std::collections::BTreeSet;

use rust_decimal::Decimal;

mod tests;

pub const SPEND_WINDOW_MS: i64 = 30 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpendRow {
    pub litellm_request_id: String,
    pub team_id: String,
    pub api_key_hash: String,
    pub spend: Decimal,
}

#[must_use]
pub fn spend_for(
    request_id: &str,
    team_id: &str,
    api_key_hash: &str,
    rows: &[SpendRow],
) -> Option<Decimal> {
    let mut matches = rows.iter().filter(|row| {
        row.litellm_request_id == request_id
            && row.team_id == team_id
            && row.api_key_hash == api_key_hash
    });
    let first = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(first.spend)
}

#[must_use]
pub fn trace_spend(
    request_ids: &[String],
    team_id: &str,
    api_key_hash: &str,
    rows: &[SpendRow],
) -> Option<Decimal> {
    let ids: BTreeSet<&str> = request_ids
        .iter()
        .map(String::as_str)
        .filter(|id| !id.is_empty())
        .collect();
    if ids.is_empty() {
        return None;
    }
    let mut total = Decimal::ZERO;
    for request_id in ids {
        let cost = spend_for(request_id, team_id, api_key_hash, rows)?;
        total = total.checked_add(cost)?;
    }
    Some(total)
}
