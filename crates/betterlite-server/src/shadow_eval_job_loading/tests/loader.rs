#![cfg(test)]

use std::sync::{Arc, Mutex};

use betterlite_db::shadow_tables::rows::JobAttemptStats;
use chrono::{DateTime, TimeZone as _, Utc};
use rust_decimal::Decimal;

use super::super::loader::ShadowJobLoader;
use super::support::{FakeCounter, FakeTables, fake_tables, frozen_clock, job_row};

type TestLoader = ShadowJobLoader<FakeTables, FakeCounter>;

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2030, 1, 1, 0, 0, 0).unwrap()
}

fn make_loader(tables: FakeTables) -> TestLoader {
    let clock = Arc::new(Mutex::new(0.0_f64));
    ShadowJobLoader::new(
        Some(Arc::new(tables)),
        FakeCounter::default(),
        frozen_clock(clock),
    )
    .unwrap()
}

fn loader_over(tables: &Arc<FakeTables>, clock: &Arc<Mutex<f64>>) -> TestLoader {
    ShadowJobLoader::new(
        Some(Arc::clone(tables)),
        FakeCounter::default(),
        frozen_clock(Arc::clone(clock)),
    )
    .unwrap()
}

#[tokio::test]
async fn shadow_eval_job_loading_groups_rows_by_target_with_attempt_stats() {
    let mut forward = job_row("forward", None);
    forward.id = "job-forward".to_owned();
    let mut reverse = job_row("reverse", Some("baseline-model"));
    reverse.id = "job-reverse".to_owned();
    let mut invalid = job_row("sideways", None);
    invalid.id = "job-invalid".to_owned();
    invalid.target_id = "key-2".to_owned();
    let tables = fake_tables(vec![forward, reverse, invalid]);
    tables.state.lock().unwrap().stats = vec![JobAttemptStats {
        job_id: "job-forward".to_owned(),
        attempts: 3,
        spend: Decimal::new(15, 1),
    }];
    let loader = make_loader(tables);
    let loaded = loader.active_jobs(now()).await;
    assert!(loaded.refilled);
    let database = loader.database.as_ref().unwrap();
    let state = database.state.lock().unwrap();
    assert_eq!(state.find_calls, 1);
    let expected = vec![
        "job-forward".to_owned(),
        "job-reverse".to_owned(),
        "job-invalid".to_owned(),
    ];
    assert_eq!(state.stats_requests.first().unwrap(), &expected);
    drop(state);
    let paired = loaded
        .jobs
        .get(&("key".to_owned(), "key-1".to_owned()))
        .unwrap();
    assert_eq!(paired.len(), 2);
    assert_eq!(paired[0].id, "job-forward");
    assert_eq!(paired[0].attempts, 3);
    assert_eq!(paired[0].spend, Decimal::new(15, 1));
    assert_eq!(paired[1].id, "job-reverse");
    assert_eq!(paired[1].attempts, 0);
    assert_eq!(paired[1].spend, Decimal::ZERO);
    assert!(
        !loaded
            .jobs
            .contains_key(&("key".to_owned(), "key-2".to_owned()))
    );
}

#[tokio::test]
async fn shadow_eval_job_loading_serves_the_second_read_from_the_cache() {
    let loader = make_loader(fake_tables(vec![job_row("forward", None)]));
    let first = loader.active_jobs(now()).await;
    let second = loader.active_jobs(now()).await;
    let calls = loader
        .database
        .as_ref()
        .unwrap()
        .state
        .lock()
        .unwrap()
        .find_calls;
    assert!(first.refilled);
    assert!(!second.refilled);
    assert_eq!(calls, 1);
    assert_eq!(first.jobs.len(), 1);
    assert_eq!(
        serde_json::to_string(&first.jobs).unwrap(),
        serde_json::to_string(&second.jobs).unwrap()
    );
    assert!(loader.cache.get().unwrap().is_some());
}

#[tokio::test]
async fn shadow_eval_job_loading_keeps_cached_costs_exact() {
    let mut row = job_row("forward", None);
    let max_budget = Decimal::new(100_000_000_000_000_005, 18);
    let spend = Decimal::new(1_000_000_000_000_000_051, 19);
    row.max_budget = Some(max_budget);
    let tables = fake_tables(vec![row]);
    tables.state.lock().unwrap().stats = vec![JobAttemptStats {
        job_id: "job-1".to_owned(),
        attempts: 2,
        spend,
    }];
    let loader = make_loader(tables);
    let first = loader.active_jobs(now()).await;
    let second = loader.active_jobs(now()).await;
    let group = ("key".to_owned(), "key-1".to_owned());
    let cached = second.jobs.get(&group).unwrap();
    assert!(!second.refilled);
    assert_eq!(first.jobs.get(&group).unwrap()[0].spend, spend);
    assert_eq!(cached[0].spend, spend);
    assert_eq!(cached[0].max_budget, Some(max_budget));
}

#[tokio::test]
async fn shadow_eval_job_loading_refills_after_the_ten_second_ttl() {
    let clock = Arc::new(Mutex::new(0.0_f64));
    let tables = Arc::new(fake_tables(vec![job_row("forward", None)]));
    let loader = loader_over(&tables, &clock);
    let first = loader.active_jobs(now()).await;
    *clock.lock().unwrap() = 10.0_f64;
    let boundary = loader.active_jobs(now()).await;
    let mut added = job_row("forward", None);
    added.id = "job-2".to_owned();
    tables.state.lock().unwrap().rows.push(added);
    *clock.lock().unwrap() = 10.5_f64;
    let refilled = loader.active_jobs(now()).await;
    assert!(first.refilled);
    assert!(!boundary.refilled);
    assert!(refilled.refilled);
    assert_eq!(
        tables.state.lock().unwrap().find_calls,
        2,
        "an expired entry reads the database again"
    );
    let group = ("key".to_owned(), "key-1".to_owned());
    assert_eq!(first.jobs.get(&group).unwrap().len(), 1);
    assert_eq!(refilled.jobs.get(&group).unwrap().len(), 2);
}

#[tokio::test]
async fn shadow_eval_job_loading_returns_empty_on_a_database_error_without_caching() {
    let tables = Arc::new(fake_tables(vec![job_row("forward", None)]));
    tables.state.lock().unwrap().fail_find = true;
    let loader = loader_over(&tables, &Arc::new(Mutex::new(0.0_f64)));
    let failed = loader.active_jobs(now()).await;
    assert!(failed.jobs.is_empty());
    assert!(!failed.refilled);
    assert!(loader.cache.get().unwrap().is_none());
    tables.state.lock().unwrap().fail_find = false;
    let retried = loader.active_jobs(now()).await;
    assert!(retried.refilled);
    assert_eq!(retried.jobs.len(), 1);
    assert_eq!(tables.state.lock().unwrap().find_calls, 2);
}

#[tokio::test]
async fn shadow_eval_job_loading_returns_empty_when_attempt_stats_fail() {
    let tables = Arc::new(fake_tables(vec![job_row("forward", None)]));
    tables.state.lock().unwrap().fail_stats = true;
    let loader = loader_over(&tables, &Arc::new(Mutex::new(0.0_f64)));
    let failed = loader.active_jobs(now()).await;
    assert!(failed.jobs.is_empty());
    assert!(!failed.refilled);
    assert!(loader.cache.get().unwrap().is_none());
    tables.state.lock().unwrap().fail_stats = false;
    let retried = loader.active_jobs(now()).await;
    assert!(retried.refilled);
    assert_eq!(retried.jobs.len(), 1);
}

#[tokio::test]
async fn shadow_eval_job_loading_without_a_database_returns_empty() {
    let loader: TestLoader = ShadowJobLoader::new(
        None,
        FakeCounter::default(),
        frozen_clock(Arc::new(Mutex::new(0.0_f64))),
    )
    .unwrap();
    let loaded = loader.active_jobs(now()).await;
    assert!(loaded.jobs.is_empty());
    assert!(!loaded.refilled);
    assert!(loader.cache.get().unwrap().is_none());
}

#[tokio::test]
async fn shadow_eval_job_loading_with_an_invalid_clock_returns_empty_without_a_read() {
    let tables = Arc::new(fake_tables(vec![job_row("forward", None)]));
    let loader = loader_over(&tables, &Arc::new(Mutex::new(f64::NAN)));
    let loaded = loader.active_jobs(now()).await;
    assert!(loaded.jobs.is_empty());
    assert!(!loaded.refilled);
    assert_eq!(tables.state.lock().unwrap().find_calls, 0);
}

#[tokio::test]
async fn shadow_eval_job_loading_caches_an_empty_read() {
    let loader = make_loader(fake_tables(Vec::new()));
    let first = loader.active_jobs(now()).await;
    let second = loader.active_jobs(now()).await;
    let database = loader.database.as_ref().unwrap();
    let state = database.state.lock().unwrap();
    assert!(first.refilled);
    assert!(!second.refilled);
    assert_eq!(state.find_calls, 1);
    assert!(state.stats_requests.is_empty());
    assert!(first.jobs.is_empty());
}
