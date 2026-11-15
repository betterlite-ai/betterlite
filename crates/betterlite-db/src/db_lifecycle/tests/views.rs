use std::time::Duration;

use rstest::rstest;
use tokio::task::yield_now;
use tokio::time::{Instant, advance, sleep};
use tokio_util::task::TaskTracker;

use super::support::{FakeFailure, capture_logs, views_harness, views_harness_with_deadline};
use crate::database_views::definitions::{RUNTIME_VIEWS, VERIFICATION_TOKEN_VIEW};
use crate::db_lifecycle::views::{
    EXPECTED_VIEWS, TOKEN_VIEW_REPAIR_SQL, ViewSetupOutcome, ViewsError, check_view_exists,
};

fn known(views: &[&str]) -> Vec<String> {
    views.iter().map(|view| String::from(*view)).collect()
}

fn others() -> Vec<&'static str> {
    EXPECTED_VIEWS
        .into_iter()
        .filter(|view| *view != VERIFICATION_TOKEN_VIEW)
        .collect()
}

#[test]
fn expected_views_cover_the_eight_runtime_views() {
    let mut expected: Vec<&str> = EXPECTED_VIEWS.to_vec();
    expected.sort_unstable();
    let mut runtime: Vec<&str> = RUNTIME_VIEWS.iter().map(|view| view.name).collect();
    runtime.sort_unstable();
    assert_eq!(expected, runtime);
}

#[tokio::test]
async fn all_views_exist_logs_and_returns() {
    let (logs, guard) = capture_logs();
    let harness = views_harness();
    *harness.catalog.state.known_views.lock().unwrap() = known(&EXPECTED_VIEWS);
    let outcome = check_view_exists(&harness.catalog, &harness.health, "public").await;
    drop(guard);
    assert!(outcome.is_ok());
    assert_eq!(
        *harness.catalog.state.schemas.lock().unwrap(),
        vec!["public".to_owned()]
    );
    assert_eq!(*harness.actions.lock().unwrap(), vec!["views".to_owned()]);
    assert!(harness.sink.row_counts.lock().unwrap().is_empty());
    assert!(
        logs.lines()
            .iter()
            .any(|line| line.contains("All necessary views exist!")),
        "{:?}",
        logs.lines()
    );
}

#[tokio::test]
async fn token_view_missing_with_other_views_checks_health_and_creates_it() {
    let harness = views_harness();
    *harness.catalog.state.known_views.lock().unwrap() = known(&others());
    let outcome = check_view_exists(&harness.catalog, &harness.health, "public").await;
    assert!(outcome.is_ok());
    assert_eq!(
        *harness.actions.lock().unwrap(),
        vec!["views".to_owned(), "health_check".to_owned()]
    );
    assert_eq!(
        harness.catalog.state.created(),
        vec![TOKEN_VIEW_REPAIR_SQL.to_owned()]
    );
    assert!(harness.catalog.state.queries.lock().unwrap().is_empty());
}

#[rstest]
#[case::team_spend("t.spend AS team_spend")]
#[case::team_max_budget("t.max_budget AS team_max_budget")]
#[case::team_model_max_budget("t.model_max_budget AS team_model_max_budget")]
#[case::team_tpm_limit("t.tpm_limit AS team_tpm_limit")]
#[case::team_rpm_limit("t.rpm_limit AS team_rpm_limit")]
#[case::team_tpd_limit("t.tpd_limit AS team_tpd_limit")]
#[tokio::test]
async fn the_repaired_token_view_projects_every_team_alias(#[case] expected: &str) {
    let harness = views_harness();
    *harness.catalog.state.known_views.lock().unwrap() = known(&others());
    let outcome = check_view_exists(&harness.catalog, &harness.health, "public").await;
    assert!(outcome.is_ok());
    let created = harness.catalog.state.created();
    let ddl = created.first().expect("the repair DDL was not executed");
    assert!(ddl.contains(expected), "{ddl}");
}

#[tokio::test]
async fn the_repaired_token_view_ignores_the_project_table() {
    let harness = views_harness();
    *harness.catalog.state.known_views.lock().unwrap() = known(&others());
    let outcome = check_view_exists(&harness.catalog, &harness.health, "public").await;
    assert!(outcome.is_ok());
    let created = harness.catalog.state.created();
    let ddl = created.first().expect("the repair DDL was not executed");
    assert!(!ddl.contains("LiteLLM_ProjectTable"), "{ddl}");
    assert!(!ddl.contains("project_alias"), "{ddl}");
}

#[tokio::test]
async fn token_view_missing_and_health_failing_does_not_create() {
    let harness = views_harness();
    *harness.catalog.state.known_views.lock().unwrap() = known(&others());
    *harness.health.outcome.lock().unwrap() = Some(FakeFailure::Dropped);
    let outcome = check_view_exists(&harness.catalog, &harness.health, "public").await;
    assert!(matches!(outcome, Err(ViewsError::Health { .. })));
    assert!(harness.catalog.state.created().is_empty());
    assert_eq!(
        *harness.actions.lock().unwrap(),
        vec!["views".to_owned(), "health_check".to_owned()]
    );
}

#[tokio::test]
async fn empty_log_with_no_views_creates_the_missing_views() {
    let harness = views_harness();
    harness.catalog.state.script_estimate(0);
    harness
        .catalog
        .state
        .script_missing_views(RUNTIME_VIEWS.len());
    harness
        .catalog
        .state
        .script_successful_creates(RUNTIME_VIEWS.len());
    let outcome = check_view_exists(&harness.catalog, &harness.health, "public").await;
    assert!(outcome.is_ok());
    let created = harness.catalog.state.created();
    assert_eq!(created.len(), RUNTIME_VIEWS.len());
    assert!(
        RUNTIME_VIEWS
            .iter()
            .all(|view| created.iter().any(|ddl| ddl == view.ddl)),
        "{created:?}"
    );
    assert_eq!(*harness.actions.lock().unwrap(), vec!["views".to_owned()]);
}

#[tokio::test]
async fn existing_log_warns_about_the_missing_views() {
    let (logs, guard) = capture_logs();
    let harness = views_harness();
    *harness.catalog.state.known_views.lock().unwrap() = known(&[VERIFICATION_TOKEN_VIEW]);
    harness.catalog.state.script_estimate(5);
    let outcome = check_view_exists(&harness.catalog, &harness.health, "public").await;
    drop(guard);
    assert!(outcome.is_ok());
    assert!(harness.catalog.state.created().is_empty());
    assert_eq!(*harness.actions.lock().unwrap(), vec!["views".to_owned()]);
    let warning = logs
        .lines()
        .into_iter()
        .find(|line| line.contains("Not all views exist in db"))
        .unwrap_or_default();
    let expected = "Not all views exist in db, needed for UI 'Usage' tab. Missing=MonthlyGlobalSpend, \
                    Last30dKeysBySpend, Last30dModelsBySpend, MonthlyGlobalSpendPerKey, \
                    MonthlyGlobalSpendPerUserPerKey, Last30dTopEndUsersSpend, DailyTagSpend. Run \
                    'create_views.py' from https://github.com/BerriAI/litellm/tree/main/db_scripts to \
                    create missing views.";
    assert!(warning.contains(expected), "{warning}");
}

#[tokio::test]
async fn catalog_failure_propagates() {
    let harness = views_harness();
    *harness.catalog.state.existing_failure.lock().unwrap() = Some(FakeFailure::Dropped);
    let outcome = check_view_exists(&harness.catalog, &harness.health, "public").await;
    assert!(matches!(outcome, Err(ViewsError::Database { .. })));
    assert_eq!(*harness.actions.lock().unwrap(), vec!["views".to_owned()]);
}

#[tokio::test(start_paused = true)]
async fn view_setup_waits_for_the_gate_table() {
    let harness = views_harness_with_deadline(Duration::from_secs(100));
    *harness.catalog.state.known_views.lock().unwrap() = known(&EXPECTED_VIEWS);
    harness.catalog.state.script_missing_gate(2);
    let (logs, guard) = capture_logs();
    let outcome = harness.setup.run().await;
    drop(guard);
    assert_eq!(outcome, ViewSetupOutcome::Ready);
    assert_eq!(
        *harness.actions.lock().unwrap(),
        vec![
            "gate".to_owned(),
            "gate".to_owned(),
            "gate".to_owned(),
            "publish".to_owned(),
            "views".to_owned()
        ]
    );
    assert_eq!(*harness.sink.row_counts.lock().unwrap(), [7]);
    let waiting = logs
        .lines()
        .iter()
        .filter(|line| {
            line.contains("Waiting for table \"LiteLLM_SpendLogs\" before creating the spend views")
        })
        .count();
    assert_eq!(waiting, 2, "{:?}", logs.lines());
}

#[tokio::test(start_paused = true)]
async fn view_setup_gives_up_when_the_table_never_appears() {
    let harness = views_harness_with_deadline(Duration::from_secs(12));
    harness.catalog.state.script_missing_gate(4);
    let (logs, guard) = capture_logs();
    let outcome = harness.setup.run().await;
    drop(guard);
    assert_eq!(outcome, ViewSetupOutcome::TimedOut);
    let actions = harness.actions.lock().unwrap().clone();
    assert_eq!(actions.len(), 4);
    assert!(actions.iter().all(|action| action == "gate"), "{actions:?}");
    assert!(
        logs.lines().iter().any(|line| line.contains(
            "Gave up creating the spend views: table \"LiteLLM_SpendLogs\" did not appear within 12.0s. \
             Run the database migrations against this database and restart the proxy."
        )),
        "{:?}",
        logs.lines()
    );
}

#[tokio::test(start_paused = true)]
async fn the_default_deadline_gives_up_after_fifteen_minutes() {
    let harness = views_harness();
    *harness.catalog.state.existing_failure.lock().unwrap() = Some(FakeFailure::Dropped);
    let started = Instant::now();
    let outcome = harness.setup.run().await;
    assert_eq!(outcome, ViewSetupOutcome::TimedOut);
    assert_eq!(started.elapsed(), Duration::from_secs(900));
}

#[tokio::test(start_paused = true)]
async fn view_setup_gives_up_when_the_attempts_keep_failing() {
    let harness = views_harness_with_deadline(Duration::from_secs(12));
    *harness.catalog.state.existing_failure.lock().unwrap() = Some(FakeFailure::Dropped);
    let (logs, guard) = capture_logs();
    let outcome = harness.setup.run().await;
    drop(guard);
    assert_eq!(outcome, ViewSetupOutcome::TimedOut);
    assert!(
        logs.lines().iter().any(|line| line.contains(
            "Gave up creating the spend views after 12.0s; the last attempt failed with: the spend view \
             catalog could not be read: the connection dropped. Fix that error and restart the proxy."
        )),
        "{:?}",
        logs.lines()
    );
    assert!(
        logs.lines().iter().any(|line| line.contains(
            "Spend view setup attempt failed, retrying until the schema settles: the spend view catalog \
             could not be read: the connection dropped"
        )),
        "{:?}",
        logs.lines()
    );
}

#[tokio::test]
async fn view_setup_start_runs_one_loop() {
    let harness = views_harness();
    *harness.catalog.state.known_views.lock().unwrap() = known(&EXPECTED_VIEWS);
    let tasks = TaskTracker::new();
    harness.setup.start(&tasks);
    harness.setup.start(&tasks);
    yield_now().await;
    assert_eq!(
        *harness.actions.lock().unwrap(),
        vec!["gate".to_owned(), "publish".to_owned(), "views".to_owned()]
    );
    harness.setup.start(&tasks);
    yield_now().await;
    assert_eq!(
        *harness.actions.lock().unwrap(),
        vec!["gate".to_owned(), "publish".to_owned(), "views".to_owned()]
    );
}

#[tokio::test(start_paused = true)]
async fn stop_ends_the_view_setup_loop() {
    let harness = views_harness_with_deadline(Duration::from_secs(1000));
    *harness.catalog.state.existing_failure.lock().unwrap() = Some(FakeFailure::Dropped);
    let tasks = TaskTracker::new();
    harness.setup.start(&tasks);
    sleep(harness.setup.poll_interval.saturating_mul(3)).await;
    let actions_before = harness.actions.lock().unwrap().clone();
    let attempts = actions_before
        .iter()
        .filter(|action| action.as_str() == "gate")
        .count();
    assert!(attempts >= 3, "{attempts}");
    assert!(harness.setup.task.stop().await);
    advance(harness.setup.poll_interval.saturating_mul(3)).await;
    yield_now().await;
    assert_eq!(*harness.actions.lock().unwrap(), actions_before);
    assert!(harness.setup.task.handle.lock().unwrap().is_none());
}
