#![cfg(test)]

use rust_decimal::Decimal;

use super::{SPEND_WINDOW_MS, SpendRow, spend_for, trace_spend};

fn row(request_id: &str, team_id: &str, api_key_hash: &str, spend: Decimal) -> SpendRow {
    SpendRow {
        litellm_request_id: request_id.to_owned(),
        team_id: team_id.to_owned(),
        api_key_hash: api_key_hash.to_owned(),
        spend,
    }
}

#[test]
fn spend_window_is_thirty_minutes_in_milliseconds() {
    assert_eq!(SPEND_WINDOW_MS, 1_800_000);
}

#[test]
fn spend_for_returns_the_cost_of_a_single_match() {
    let rows = vec![row("req-1", "team-1", "hash-1", Decimal::new(500, 2))];
    assert_eq!(
        spend_for("req-1", "team-1", "hash-1", &rows),
        Some(Decimal::new(500, 2))
    );
}

#[test]
fn spend_for_returns_none_without_a_match() {
    let rows = vec![row("req-1", "team-1", "hash-1", Decimal::ONE)];
    assert_eq!(spend_for("req-2", "team-1", "hash-1", &rows), None);
}

#[test]
fn spend_for_returns_none_on_more_than_one_match() {
    let rows = vec![
        row("req-1", "team-1", "hash-1", Decimal::ONE),
        row("req-1", "team-1", "hash-1", Decimal::TWO),
    ];
    assert_eq!(spend_for("req-1", "team-1", "hash-1", &rows), None);
}

#[test]
fn trace_spend_sums_costs_when_every_request_id_has_an_entry() {
    let rows = vec![
        row("req-1", "team-1", "hash-1", Decimal::ONE),
        row("req-2", "team-1", "hash-1", Decimal::TWO),
    ];
    let ids = vec!["req-1".to_owned(), "req-2".to_owned()];
    assert_eq!(
        trace_spend(&ids, "team-1", "hash-1", &rows),
        Some(Decimal::new(3, 0))
    );
}

#[test]
fn trace_spend_is_none_when_one_request_id_has_no_entry() {
    let rows = vec![row("req-1", "team-1", "hash-1", Decimal::ONE)];
    let ids = vec!["req-1".to_owned(), "req-2".to_owned()];
    assert_eq!(trace_spend(&ids, "team-1", "hash-1", &rows), None);
}

#[test]
fn trace_spend_is_none_for_no_request_ids() {
    assert_eq!(trace_spend(&[], "team-1", "hash-1", &[]), None);
}

#[test]
fn trace_spend_ignores_empty_request_ids() {
    let ids = vec![String::new()];
    assert_eq!(trace_spend(&ids, "team-1", "hash-1", &[]), None);
}
