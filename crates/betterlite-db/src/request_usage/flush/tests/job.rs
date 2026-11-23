use std::time::Duration;

use crate::request_usage::flush::{SpendLogsJobError, queued_spend_transactions};
use crate::shadow_tables::rows::FunnelStage;
use crate::spend_log_storage::rows::QueuePosition;

use super::fakes::ScriptedFailure;
use super::support::{
    Harness, baseline_record, fill_queue, guardrail_row, model_transaction, tool_transaction,
    turn_transaction, unreadable_guardrail_row,
};

#[tokio::test]
async fn sums_every_queue_and_the_funnel_in_the_census() {
    let harness = Harness::new();
    harness
        .logs
        .enqueue(&[guardrail_row("r1")], QueuePosition::Tail);
    harness
        .tool_usage
        .push(tool_transaction("t1"))
        .await
        .unwrap();
    harness.model_usage.push(model_transaction()).await.unwrap();
    harness
        .turns_queue
        .push(turn_transaction("sess-1"))
        .await
        .unwrap();
    harness.baseline.push(baseline_record()).await.unwrap();
    harness.funnel.record("job-1", FunnelStage::Shed);
    let job = harness.job();
    assert_eq!(queued_spend_transactions(&job.queues), 6);
    drop(harness.logs.dequeue(1));
    assert_eq!(queued_spend_transactions(&job.queues), 5);
}

#[tokio::test]
async fn an_empty_census_still_flushes_the_baseline_projection() {
    let harness = Harness::new();
    harness.accounting.mark_dirty("scope-1");
    harness.job().update_spend_logs_job().await.unwrap();
    assert!(harness.trace.ran("baseline"));
    assert!(!harness.trace.ran("log"));
    assert_eq!(harness.log_writer.calls(), 0);
}

#[tokio::test]
async fn a_baseline_only_backlog_runs_the_job() {
    let harness = Harness::new();
    harness.baseline.push(baseline_record()).await.unwrap();
    harness.accounting.mark_dirty("scope-1");
    harness.job().update_spend_logs_job().await.unwrap();
    assert_eq!(harness.accounting.appended(), 1);
    assert!(!harness.trace.ran("log"));
}

#[tokio::test]
async fn a_funnel_only_backlog_runs_the_job() {
    let harness = Harness::new();
    harness.funnel.record("job-1", FunnelStage::Shed);
    harness.job().update_spend_logs_job().await.unwrap();
    let flushed = harness.shadows.funnel();
    assert_eq!(flushed.len(), 1);
    assert_eq!(flushed[0].0, "job-1");
    assert_eq!(flushed[0].1.total(), 1);
    assert!(!harness.trace.ran("log"));
}

#[tokio::test]
async fn a_pass_waits_for_the_write_lock() {
    let harness = Harness::new();
    harness
        .logs
        .enqueue(&[guardrail_row("r1")], QueuePosition::Tail);
    let held = harness.write_lock.lock().await;
    assert!(
        tokio::time::timeout(
            Duration::from_millis(20),
            harness.job().update_spend_logs_job()
        )
        .await
        .is_err()
    );
    assert_eq!(harness.log_writer.rows().len(), 0);
    drop(held);
    harness.job().update_spend_logs_job().await.unwrap();
    assert_eq!(harness.log_writer.rows().len(), 1);
}

#[tokio::test]
async fn orders_every_derived_stage_after_the_log_write() {
    let harness = Harness::new();
    harness
        .logs
        .enqueue(&[guardrail_row("r1")], QueuePosition::Tail);
    harness
        .tool_usage
        .push(tool_transaction("r1"))
        .await
        .unwrap();
    harness.model_usage.push(model_transaction()).await.unwrap();
    harness
        .turns_queue
        .push(turn_transaction("sess-1"))
        .await
        .unwrap();
    harness.baseline.push(baseline_record()).await.unwrap();
    harness.accounting.mark_dirty("scope-1");
    harness.funnel.record("job-1", FunnelStage::Shed);
    harness.job().update_spend_logs_job().await.unwrap();
    assert_eq!(
        harness.trace.stage_order(),
        vec![
            "log",
            "guardrail",
            "tool",
            "model",
            "baseline",
            "turn",
            "funnel"
        ]
    );
    assert_eq!(harness.log_writer.rows().len(), 1);
    assert_eq!(harness.guardrail_sink.rows().len(), 1);
    assert_eq!(harness.tool_writer.index_rows(), 1);
    assert_eq!(harness.model_writer.rollups().len(), 1);
    assert_eq!(harness.accounting.appended(), 1);
    assert_eq!(harness.turn_writer.turns().len(), 1);
    assert_eq!(harness.shadows.funnel().len(), 1);
}

#[tokio::test]
async fn a_failed_log_write_stops_the_pass_and_returns_the_rows() {
    let harness = Harness::new();
    harness
        .log_writer
        .fail_next(ScriptedFailure::DATABASE_ERROR);
    harness.logs.enqueue(
        &[guardrail_row("r1"), guardrail_row("r2")],
        QueuePosition::Tail,
    );
    harness
        .tool_usage
        .push(tool_transaction("r1"))
        .await
        .unwrap();
    let error = harness.job().update_spend_logs_job().await.unwrap_err();
    assert!(matches!(error, SpendLogsJobError::Logs(_)));
    assert_eq!(harness.logs.len(), 2);
    assert_eq!(harness.tool_usage.len(), 1);
    assert!(!harness.trace.ran("guardrail"));
    assert!(!harness.trace.ran("tool"));
}

#[tokio::test]
async fn a_cancelled_pass_requeues_the_popped_rows_once() {
    let harness = Harness::new();
    harness.shutdown.cancel();
    harness
        .logs
        .enqueue(&[guardrail_row("r1")], QueuePosition::Tail);
    let error = harness.job().run_spend_logs_job().await.unwrap_err();
    assert!(matches!(error, SpendLogsJobError::Cancelled));
    assert_eq!(harness.logs.len(), 1);
    assert_eq!(harness.log_writer.calls(), 0);
    assert!(!harness.trace.ran("guardrail"));
}

#[tokio::test]
async fn a_dropped_write_returns_the_popped_rows_to_the_queue_head() {
    let harness = Harness::new();
    harness.log_writer.stall();
    harness
        .logs
        .enqueue(&[guardrail_row("r1")], QueuePosition::Tail);
    let job = harness.job();
    let mut write = Box::pin(job.write_popped_logs());
    assert!(
        tokio::time::timeout(Duration::from_millis(20), write.as_mut())
            .await
            .is_err()
    );
    assert_eq!(harness.logs.len(), 0);
    assert_eq!(harness.log_writer.calls(), 1);
    drop(write);
    assert_eq!(harness.logs.len(), 1);
}

#[tokio::test]
async fn drains_10000_logs_in_one_pass() {
    let harness = Harness::new();
    let rows: Vec<_> = (0_u32..10_001)
        .map(|index| guardrail_row(&format!("log-{index}")))
        .collect();
    harness.logs.enqueue(&rows, QueuePosition::Tail);
    harness.job().update_spend_logs_job().await.unwrap();
    assert_eq!(harness.logs.len(), 1);
    assert_eq!(harness.log_writer.rows().len(), 10_000);
}

#[tokio::test]
async fn drains_10000_tool_records_and_every_model_record_in_one_pass() {
    let harness = Harness::new();
    fill_queue(&harness.tool_usage, 10_001, |index| {
        tool_transaction(&format!("tool-{index}"))
    })
    .await;
    fill_queue(&harness.model_usage, 10_001, |_| model_transaction()).await;
    harness.job().update_spend_logs_job().await.unwrap();
    assert_eq!(harness.tool_usage.len(), 1);
    assert!(harness.model_usage.is_empty());
    assert_eq!(harness.tool_writer.index_rows(), 10_000);
    let tool_rollups = harness.tool_writer.rollups();
    assert_eq!(tool_rollups.len(), 1);
    assert_eq!(tool_rollups[0].request_count, 10_000);
    let rollups = harness.model_writer.rollups();
    assert_eq!(rollups.len(), 1);
    assert_eq!(rollups[0].request_count, 10_001);
}

#[tokio::test]
async fn drains_10000_turns_in_one_pass() {
    let harness = Harness::new();
    fill_queue(&harness.turns_queue, 10_001, |index| {
        turn_transaction(&format!("sess-{index}"))
    })
    .await;
    harness.job().update_spend_logs_job().await.unwrap();
    assert_eq!(harness.turns_queue.len(), 1);
    assert_eq!(harness.turn_writer.turns().len(), 10_000);
}

#[tokio::test]
async fn a_failed_tool_flush_leaves_the_later_stages_running() {
    let harness = Harness::new();
    harness.tool_writer.fail();
    harness
        .tool_usage
        .push(tool_transaction("t1"))
        .await
        .unwrap();
    harness.model_usage.push(model_transaction()).await.unwrap();
    harness.funnel.record("job-1", FunnelStage::Shed);
    harness.job().update_spend_logs_job().await.unwrap();
    assert!(harness.trace.ran("tool"));
    assert_eq!(harness.tool_usage.len(), 0);
    assert_eq!(harness.model_writer.rollups().len(), 1);
    assert_eq!(harness.shadows.funnel().len(), 1);
}

#[tokio::test]
async fn a_failed_model_flush_leaves_the_later_stages_running() {
    let harness = Harness::new();
    harness.model_writer.fail();
    harness.model_usage.push(model_transaction()).await.unwrap();
    harness
        .turns_queue
        .push(turn_transaction("sess-1"))
        .await
        .unwrap();
    harness.funnel.record("job-1", FunnelStage::Shed);
    harness.job().update_spend_logs_job().await.unwrap();
    assert!(harness.trace.ran("model"));
    assert!(harness.model_usage.is_empty());
    assert_eq!(harness.turn_writer.turns().len(), 1);
    assert_eq!(harness.shadows.funnel().len(), 1);
}

#[tokio::test]
async fn a_failed_guardrail_write_does_not_stop_the_pass() {
    let harness = Harness::new();
    harness.guardrail_sink.fail();
    harness
        .logs
        .enqueue(&[guardrail_row("r1")], QueuePosition::Tail);
    harness.funnel.record("job-1", FunnelStage::Shed);
    harness.job().update_spend_logs_job().await.unwrap();
    assert!(harness.trace.ran("guardrail"));
    assert_eq!(harness.shadows.funnel().len(), 1);
}

#[tokio::test]
async fn an_unreadable_row_skips_only_its_guardrail_usage() {
    let harness = Harness::new();
    harness.logs.enqueue(
        &[unreadable_guardrail_row("r1"), guardrail_row("r2")],
        QueuePosition::Tail,
    );
    harness.job().update_spend_logs_job().await.unwrap();
    assert_eq!(harness.log_writer.rows().len(), 2);
    let rows = harness.guardrail_sink.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].request_id, "r2");
}
