#![cfg(test)]

use betterlite_db::shadow_tables::funnel::FunnelQueue;
use betterlite_db::shadow_tables::rows::{AttemptOutcome, FunnelStage};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::{PipelineDeps, run_shadow_eval};
use crate::shadow_evaluation::advisory_budget::AdvisoryDeps;
use crate::shadow_evaluation::tests::{
    Alerts, Answer, Attempts, CallsFake, FunnelLog, JobSpendFake, JobSpendRead, SampleFixture,
    SpendAnswer, SpendFake, SpendLog, SpendScript, TeamAnswer, TeamsFake, call_failure, decimal,
    eval_job, hook_metadata, judge_verdict, key_auth, now, shadow_reply,
};

fn advisory_deps<'deps>(
    spend: &'deps SpendFake,
    teams: &'deps TeamsFake,
    alerts: &'deps Alerts,
) -> AdvisoryDeps<'deps> {
    AdvisoryDeps {
        spend,
        teams,
        key_alerts: Some(alerts),
        team_alerts: Some(alerts),
        now: now(),
    }
}

fn spend_key(job_id: &str) -> String {
    format!("spend:shadow_eval:{job_id}")
}

fn withheld(job_id: &str) -> Vec<(String, FunnelStage)> {
    vec![(job_id.to_owned(), FunnelStage::Withheld)]
}

fn over_budget_metadata() -> IndexMap<String, Json> {
    hook_metadata(key_auth(Some("1"), "1", None))
}

#[tokio::test]
async fn a_pod_without_a_database_withholds_without_reading_anything() {
    let job = eval_job("job-1", &["router-a"], Some(decimal("5")));
    let mut fixture = SampleFixture::new();
    fixture.parent_metadata = over_budget_metadata();
    let attempts = Attempts::new();
    let counter = SpendLog::new();
    let calls = CallsFake::new(Vec::new(), Vec::new());
    let funnel = FunnelLog::new();
    let spend = SpendFake::new(
        SpendScript::Value(decimal("1")),
        SpendScript::Value(decimal("0")),
    );
    let teams = TeamsFake::new(TeamAnswer::NoDatabase);
    let alerts = Alerts::default();
    let advisory = advisory_deps(&spend, &teams, &alerts);
    let job_spend = JobSpendFake::new(SpendAnswer::Unreadable);
    let deps: PipelineDeps<'_, Attempts, SpendLog> = PipelineDeps {
        database: None,
        counter: &counter,
        funnel: &funnel,
        advisory: &advisory,
        spend: &job_spend,
        calls: &calls,
    };
    run_shadow_eval(&job, &fixture.sample(), &deps).await;
    assert_eq!(funnel.events(), withheld("job-1"));
    assert!(attempts.rows().is_empty());
    assert!(calls.shadow_calls().is_empty());
    assert!(job_spend.reads().is_empty());
    assert!(spend.reads().is_empty());
}

#[tokio::test]
async fn an_over_budget_caller_withholds_before_the_job_budget_read() {
    let job = eval_job("job-1", &["router-a"], Some(decimal("5")));
    let mut fixture = SampleFixture::new();
    fixture.parent_metadata = over_budget_metadata();
    let attempts = Attempts::new();
    let counter = SpendLog::new();
    let calls = CallsFake::new(Vec::new(), Vec::new());
    let funnel = FunnelLog::new();
    let spend = SpendFake::new(
        SpendScript::Value(decimal("1")),
        SpendScript::Value(decimal("0")),
    );
    let teams = TeamsFake::new(TeamAnswer::NoDatabase);
    let alerts = Alerts::default();
    let advisory = advisory_deps(&spend, &teams, &alerts);
    let job_spend = JobSpendFake::new(SpendAnswer::Unreadable);
    let deps = PipelineDeps {
        database: Some(&attempts),
        counter: &counter,
        funnel: &funnel,
        advisory: &advisory,
        spend: &job_spend,
        calls: &calls,
    };
    run_shadow_eval(&job, &fixture.sample(), &deps).await;
    assert_eq!(funnel.events(), withheld("job-1"));
    assert!(job_spend.reads().is_empty());
    assert!(calls.shadow_calls().is_empty());
    assert!(attempts.rows().is_empty());
}

#[tokio::test]
async fn an_unreadable_job_spend_withholds() {
    let mut job = eval_job("job-1", &["router-a"], Some(decimal("5")));
    job.spend = decimal("2");
    let fixture = SampleFixture::new();
    let attempts = Attempts::new();
    let counter = SpendLog::new();
    let calls = CallsFake::new(Vec::new(), Vec::new());
    let funnel = FunnelLog::new();
    let spend = SpendFake::new(
        SpendScript::Value(decimal("0")),
        SpendScript::Value(decimal("0")),
    );
    let teams = TeamsFake::new(TeamAnswer::NoDatabase);
    let alerts = Alerts::default();
    let advisory = advisory_deps(&spend, &teams, &alerts);
    let job_spend = JobSpendFake::new(SpendAnswer::Unreadable);
    let deps = PipelineDeps {
        database: Some(&attempts),
        counter: &counter,
        funnel: &funnel,
        advisory: &advisory,
        spend: &job_spend,
        calls: &calls,
    };
    run_shadow_eval(&job, &fixture.sample(), &deps).await;
    assert_eq!(funnel.events(), withheld("job-1"));
    assert_eq!(
        job_spend.reads(),
        vec![JobSpendRead {
            counter_key: spend_key("job-1"),
            fallback_spend: decimal("2"),
            max_budget: decimal("5"),
        }]
    );
    assert!(calls.shadow_calls().is_empty());
}

#[tokio::test]
async fn job_spend_at_the_cap_withholds() {
    let job = eval_job("job-1", &["router-a"], Some(decimal("5")));
    let fixture = SampleFixture::new();
    let attempts = Attempts::new();
    let counter = SpendLog::new();
    let calls = CallsFake::new(Vec::new(), Vec::new());
    let funnel = FunnelLog::new();
    let spend = SpendFake::new(
        SpendScript::Value(decimal("0")),
        SpendScript::Value(decimal("0")),
    );
    let teams = TeamsFake::new(TeamAnswer::NoDatabase);
    let alerts = Alerts::default();
    let advisory = advisory_deps(&spend, &teams, &alerts);
    let job_spend = JobSpendFake::new(SpendAnswer::Spend(decimal("5")));
    let deps = PipelineDeps {
        database: Some(&attempts),
        counter: &counter,
        funnel: &funnel,
        advisory: &advisory,
        spend: &job_spend,
        calls: &calls,
    };
    run_shadow_eval(&job, &fixture.sample(), &deps).await;
    assert_eq!(funnel.events(), withheld("job-1"));
    assert_eq!(job_spend.reads().len(), 1);
    assert!(calls.shadow_calls().is_empty());
}

#[tokio::test]
async fn job_spend_below_the_cap_runs_every_arm() {
    let job = eval_job("job-1", &["router-a", "router-b"], Some(decimal("5")));
    let fixture = SampleFixture::new();
    let attempts = Attempts::new();
    let counter = SpendLog::new();
    let calls = CallsFake::new(
        vec![Answer::Reply(shadow_reply()), Answer::Reply(shadow_reply())],
        vec![
            Answer::Reply(judge_verdict("shadow")),
            Answer::Reply(judge_verdict("real")),
        ],
    );
    let funnel = FunnelLog::new();
    let spend = SpendFake::new(
        SpendScript::Value(decimal("0")),
        SpendScript::Value(decimal("0")),
    );
    let teams = TeamsFake::new(TeamAnswer::NoDatabase);
    let alerts = Alerts::default();
    let advisory = advisory_deps(&spend, &teams, &alerts);
    let job_spend = JobSpendFake::new(SpendAnswer::Spend(decimal("4.99")));
    let deps = PipelineDeps {
        database: Some(&attempts),
        counter: &counter,
        funnel: &funnel,
        advisory: &advisory,
        spend: &job_spend,
        calls: &calls,
    };
    run_shadow_eval(&job, &fixture.sample(), &deps).await;
    assert!(funnel.events().is_empty());
    assert_eq!(
        calls.shadow_targets(),
        vec!["router-a".to_owned(), "router-b".to_owned()]
    );
    let rows = attempts.rows();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].outcome, AttemptOutcome::Shadow);
    assert_eq!(rows[1].outcome, AttemptOutcome::Real);
}

#[tokio::test]
async fn a_job_without_a_budget_never_reads_the_job_counter() {
    let job = eval_job("job-1", &["router-a"], None);
    let fixture = SampleFixture::new();
    let attempts = Attempts::new();
    let counter = SpendLog::new();
    let calls = CallsFake::new(
        vec![Answer::Reply(shadow_reply())],
        vec![Answer::Reply(judge_verdict("tie"))],
    );
    let funnel = FunnelLog::new();
    let spend = SpendFake::new(
        SpendScript::Value(decimal("0")),
        SpendScript::Value(decimal("0")),
    );
    let teams = TeamsFake::new(TeamAnswer::NoDatabase);
    let alerts = Alerts::default();
    let advisory = advisory_deps(&spend, &teams, &alerts);
    let job_spend = JobSpendFake::new(SpendAnswer::Unreadable);
    let deps = PipelineDeps {
        database: Some(&attempts),
        counter: &counter,
        funnel: &funnel,
        advisory: &advisory,
        spend: &job_spend,
        calls: &calls,
    };
    run_shadow_eval(&job, &fixture.sample(), &deps).await;
    assert!(job_spend.reads().is_empty());
    assert_eq!(attempts.rows().len(), 1);
}

#[tokio::test]
async fn every_arm_runs_in_order() {
    let job = eval_job("job-1", &["router-a", "router-b", "router-c"], None);
    let fixture = SampleFixture::new();
    let attempts = Attempts::new();
    let counter = SpendLog::new();
    let calls = CallsFake::new(
        vec![
            Answer::Reply(shadow_reply()),
            Answer::Reply(shadow_reply()),
            Answer::Reply(shadow_reply()),
        ],
        vec![
            Answer::Reply(judge_verdict("tie")),
            Answer::Reply(judge_verdict("tie")),
            Answer::Reply(judge_verdict("tie")),
        ],
    );
    let funnel = FunnelLog::new();
    let spend = SpendFake::new(
        SpendScript::Value(decimal("0")),
        SpendScript::Value(decimal("0")),
    );
    let teams = TeamsFake::new(TeamAnswer::NoDatabase);
    let alerts = Alerts::default();
    let advisory = advisory_deps(&spend, &teams, &alerts);
    let job_spend = JobSpendFake::new(SpendAnswer::Spend(Decimal::ZERO));
    let deps = PipelineDeps {
        database: Some(&attempts),
        counter: &counter,
        funnel: &funnel,
        advisory: &advisory,
        spend: &job_spend,
        calls: &calls,
    };
    run_shadow_eval(&job, &fixture.sample(), &deps).await;
    assert_eq!(
        calls.shadow_targets(),
        vec![
            "router-a".to_owned(),
            "router-b".to_owned(),
            "router-c".to_owned()
        ]
    );
    let rows = attempts.rows();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].router_name, "router-a");
    assert_eq!(rows[1].router_name, "router-b");
    assert_eq!(rows[2].router_name, "router-c");
}

#[tokio::test]
async fn a_failed_arm_does_not_silence_the_next() {
    let job = eval_job("job-1", &["router-a", "router-b"], None);
    let fixture = SampleFixture::new();
    let attempts = Attempts::new();
    let counter = SpendLog::new();
    let calls = CallsFake::new(
        vec![
            Answer::Failed(call_failure(
                "shadow router call failed: timeout",
                "0.3",
                "0.05",
            )),
            Answer::Reply(shadow_reply()),
        ],
        vec![Answer::Reply(judge_verdict("shadow"))],
    );
    let funnel = FunnelLog::new();
    let spend = SpendFake::new(
        SpendScript::Value(decimal("0")),
        SpendScript::Value(decimal("0")),
    );
    let teams = TeamsFake::new(TeamAnswer::NoDatabase);
    let alerts = Alerts::default();
    let advisory = advisory_deps(&spend, &teams, &alerts);
    let job_spend = JobSpendFake::new(SpendAnswer::Spend(Decimal::ZERO));
    let deps = PipelineDeps {
        database: Some(&attempts),
        counter: &counter,
        funnel: &funnel,
        advisory: &advisory,
        spend: &job_spend,
        calls: &calls,
    };
    run_shadow_eval(&job, &fixture.sample(), &deps).await;
    assert_eq!(
        calls.shadow_targets(),
        vec!["router-a".to_owned(), "router-b".to_owned()]
    );
    assert_eq!(calls.judge_calls().len(), 1);
    assert_eq!(calls.judge_calls()[0].shadow_text, "shadow answer");
    let rows = attempts.rows();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].outcome, AttemptOutcome::Error);
    assert_eq!(rows[1].outcome, AttemptOutcome::Shadow);
    assert_eq!(
        counter.entries(),
        vec![
            (spend_key("job-1"), decimal("0.35")),
            (spend_key("job-1"), decimal("1.75")),
        ]
    );
}

#[tokio::test]
async fn the_production_funnel_queue_counts_a_withheld_sample() {
    let job = eval_job("job-1", &["router-a"], None);
    let fixture = SampleFixture::new();
    let counter = SpendLog::new();
    let calls = CallsFake::new(Vec::new(), Vec::new());
    let funnel = FunnelQueue::default();
    let spend = SpendFake::new(
        SpendScript::Value(decimal("1")),
        SpendScript::Value(decimal("0")),
    );
    let teams = TeamsFake::new(TeamAnswer::NoDatabase);
    let alerts = Alerts::default();
    let advisory = advisory_deps(&spend, &teams, &alerts);
    let job_spend = JobSpendFake::new(SpendAnswer::Spend(Decimal::ZERO));
    let deps: PipelineDeps<'_, Attempts, SpendLog> = PipelineDeps {
        database: None,
        counter: &counter,
        funnel: &funnel,
        advisory: &advisory,
        spend: &job_spend,
        calls: &calls,
    };
    run_shadow_eval(&job, &fixture.sample(), &deps).await;
    assert_eq!(funnel.pending_events(), 1_i64);
}
