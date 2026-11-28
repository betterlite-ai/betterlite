#![cfg(test)]

use betterlite_config::sections::prometheus_budget_refresh::Settings;

use super::{BudgetRefreshJob, JOB_ID, budget_refresh_job};
use crate::prometheus_budget_refresh::tests::support::capture_logs;

fn enabled(interval_minutes: i64) -> Settings {
    Settings {
        initialize_budget_metrics: true,
        refresh_interval_minutes: interval_minutes,
    }
}

#[test]
fn a_disabled_setting_returns_no_job() {
    let (logs, _guard) = capture_logs();

    let job = budget_refresh_job(&Settings::default(), &["scheduler"]);

    assert!(job.is_none());
    assert!(!logs.text().contains("prometheus loggers"));
}

#[test]
fn no_loggers_returns_no_job_and_logs_the_count() {
    let (logs, _guard) = capture_logs();
    let loggers: [&str; 0] = [];

    let job = budget_refresh_job(&enabled(5), &loggers);

    assert!(job.is_none());
    assert!(
        logs.text().contains("found 0 prometheus loggers"),
        "{}",
        logs.text()
    );
}

#[test]
fn the_first_logger_and_the_interval_go_into_the_job() {
    let (logs, _guard) = capture_logs();
    let loggers = ["scheduler-one", "scheduler-two"];

    let job = budget_refresh_job(&enabled(7), &loggers).unwrap();

    assert_eq!(
        job,
        BudgetRefreshJob {
            id: JOB_ID,
            interval_minutes: 7,
            logger: "scheduler-one",
        }
    );
    let captured = logs.text();
    assert!(
        captured.contains("found 2 prometheus loggers"),
        "{captured}"
    );
    assert!(
        captured.contains(
            "Initializing remaining budget metrics as a cron job executing every 7 minutes"
        ),
        "{captured}"
    );
}

#[test]
fn the_job_id_is_the_litellm_job_name() {
    assert_eq!(JOB_ID, "prometheus_budget_metrics_job");
}
