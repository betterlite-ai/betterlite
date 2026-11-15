#![cfg(test)]

use betterlite_db::shadow_tables::rows::{AttemptOutcome, ShadowAttemptRow};
use rust_decimal::Decimal;

use super::super::attempt::{attempt_tier, record_attempt};
use super::super::jobs::ShadowEvalDirection;
use super::support::{FakeCounter, FakeTables, log_entries, tables_and_counter};

fn attempt() -> ShadowAttemptRow {
    ShadowAttemptRow {
        job_id: "job-a".to_owned(),
        request_id: "req-1".to_owned(),
        router_name: "router-a".to_owned(),
        outcome: AttemptOutcome::Shadow,
        tier: Some("tier-shadow".to_owned()),
        real_model: Some("gpt-4o".to_owned()),
        shadow_model: Some("gpt-4o-mini".to_owned()),
        confidence: Some(Decimal::new(875, 3)),
        judge_cost: Decimal::new(25, 2),
        shadow_cost: Decimal::new(5, 1),
        shadow_classifier_cost: Decimal::new(125, 3),
        real_cost: Some(Decimal::new(75, 2)),
        real_classifier_cost: Decimal::new(625, 4),
        real_cache_hit: false,
        error: None,
    }
}

fn inserted(tables: &FakeTables) -> Vec<ShadowAttemptRow> {
    tables.state.lock().unwrap().inserts.clone()
}

fn spent(counter: &FakeCounter) -> Decimal {
    counter.state.lock().unwrap().spend
}

#[tokio::test]
async fn shadow_eval_attempt_spend_counter_write_orders_before_insert() {
    let (tables, counter) = tables_and_counter();
    record_attempt(Some(&tables), &counter, attempt()).await;
    assert_eq!(
        log_entries(&tables.events),
        vec![
            "write:spend:shadow_eval:job-a:0.875".to_owned(),
            "insert:req-1".to_owned(),
        ]
    );
    assert_eq!(spent(&counter), Decimal::new(875, 3));
    let rows = inserted(&tables);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].judge_cost, Decimal::new(25, 2));
    assert_eq!(rows[0].shadow_cost, Decimal::new(5, 1));
    assert_eq!(rows[0].shadow_classifier_cost, Decimal::new(125, 3));
    assert_eq!(rows[0].real_cost, Some(Decimal::new(75, 2)));
    assert_eq!(rows[0].real_classifier_cost, Decimal::new(625, 4));
    assert_eq!(rows[0].confidence, Some(Decimal::new(875, 3)));
    assert_eq!(rows[0].outcome, AttemptOutcome::Shadow);
    assert_eq!(rows[0].real_model, Some("gpt-4o".to_owned()));
    assert!(!rows[0].real_cache_hit);
    assert_eq!(rows[0].error, None);
}

#[tokio::test]
async fn shadow_eval_attempt_without_eval_spend_writes_no_counter() {
    let (tables, counter) = tables_and_counter();
    let mut row = attempt();
    row.judge_cost = Decimal::ZERO;
    row.shadow_cost = Decimal::ZERO;
    row.shadow_classifier_cost = Decimal::ZERO;
    record_attempt(Some(&tables), &counter, row).await;
    assert_eq!(log_entries(&tables.events), vec!["insert:req-1".to_owned()]);
    assert_eq!(spent(&counter), Decimal::ZERO);
    assert_eq!(inserted(&tables).len(), 1);
}

#[tokio::test]
async fn shadow_eval_attempt_counter_failure_still_inserts_the_row() {
    let (tables, counter) = tables_and_counter();
    counter.state.lock().unwrap().fail_writes = true;
    record_attempt(Some(&tables), &counter, attempt()).await;
    assert_eq!(
        log_entries(&tables.events),
        vec![
            "write:spend:shadow_eval:job-a:0.875".to_owned(),
            "insert:req-1".to_owned(),
        ]
    );
    assert_eq!(inserted(&tables).len(), 1);
}

#[tokio::test]
async fn shadow_eval_attempt_without_a_database_still_writes_the_counter() {
    let counter = FakeCounter::default();
    record_attempt::<FakeTables, FakeCounter>(None, &counter, attempt()).await;
    assert_eq!(
        log_entries(&counter.events),
        vec!["write:spend:shadow_eval:job-a:0.875".to_owned()]
    );
    assert_eq!(spent(&counter), Decimal::new(875, 3));
}

#[tokio::test]
async fn shadow_eval_attempt_insert_failure_is_swallowed() {
    let (tables, counter) = tables_and_counter();
    tables.state.lock().unwrap().fail_insert = true;
    record_attempt(Some(&tables), &counter, attempt()).await;
    assert_eq!(spent(&counter), Decimal::new(875, 3));
    assert_eq!(inserted(&tables).len(), 1);
}

#[tokio::test]
async fn shadow_eval_attempt_error_is_truncated_to_five_hundred_chars() {
    let (tables, counter) = tables_and_counter();
    let mut kept = attempt();
    kept.request_id = "req-kept".to_owned();
    kept.error = Some("x".repeat(500));
    let mut cut = attempt();
    cut.request_id = "req-cut".to_owned();
    cut.error = Some("x".repeat(501));
    let mut wide = attempt();
    wide.request_id = "req-wide".to_owned();
    wide.error = Some("é".repeat(600));
    record_attempt(Some(&tables), &counter, kept).await;
    record_attempt(Some(&tables), &counter, cut).await;
    record_attempt(Some(&tables), &counter, wide).await;
    let rows = inserted(&tables);
    assert_eq!(rows.len(), 3);
    let kept_error = rows[0].error.as_ref().unwrap();
    assert_eq!(kept_error.chars().count(), 500);
    let cut_error = rows[1].error.as_ref().unwrap();
    assert_eq!(cut_error, &"x".repeat(500));
    let wide_error = rows[2].error.as_ref().unwrap();
    assert_eq!(wide_error.chars().count(), 500);
    assert_eq!(wide_error.len(), 1000);
}

#[tokio::test]
async fn shadow_eval_attempt_empty_error_and_real_model_are_stored_as_none() {
    let (tables, counter) = tables_and_counter();
    let mut row = attempt();
    row.error = Some(String::new());
    row.real_model = Some(String::new());
    record_attempt(Some(&tables), &counter, row).await;
    let rows = inserted(&tables);
    assert_eq!(rows[0].error, None);
    assert_eq!(rows[0].real_model, None);
}

#[test]
fn shadow_eval_attempt_tier_follows_the_direction() {
    let control = Some("tier-control".to_owned());
    let shadow = Some("tier-shadow".to_owned());
    assert_eq!(
        attempt_tier(
            ShadowEvalDirection::Reverse,
            control.clone(),
            shadow.clone()
        ),
        Some("tier-control".to_owned())
    );
    assert_eq!(
        attempt_tier(ShadowEvalDirection::Forward, control, shadow),
        Some("tier-shadow".to_owned())
    );
    assert_eq!(
        attempt_tier(
            ShadowEvalDirection::Reverse,
            None,
            Some("tier-shadow".to_owned())
        ),
        None
    );
    assert_eq!(
        attempt_tier(
            ShadowEvalDirection::Forward,
            Some("tier-control".to_owned()),
            None
        ),
        None
    );
}
