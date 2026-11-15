#![cfg(test)]

use std::sync::{Arc, Mutex, PoisonError};

use chrono::{DateTime, Utc};

use super::super::funnel::FunnelQueue;
use super::super::rows::{
    FunnelCounters, FunnelStage, JobAttemptStats, ShadowAttemptRow, ShadowFuture, ShadowJobRow,
    ShadowTables,
};

#[derive(Debug, thiserror::Error)]
#[error("scripted funnel failure")]
struct Scripted;

#[derive(Default)]
struct RecorderState {
    upserts: Vec<(String, FunnelCounters)>,
    fail_for: Vec<String>,
    interleave: Option<(String, Arc<FunnelQueue>)>,
}

#[derive(Default)]
struct FunnelRecorder {
    state: Mutex<RecorderState>,
}

impl FunnelRecorder {
    fn fail_for(&self, job_id: &str) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let failed = job_id.to_owned();
        state.fail_for.push(failed);
    }

    // record into the queue from inside the flush, the pod-local interleaving a timer never sees
    fn interleave(&self, job_id: &str, queue: &Arc<FunnelQueue>) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let late = job_id.to_owned();
        state.interleave = Some((late, Arc::clone(queue)));
    }

    fn upserts(&self) -> Vec<(String, FunnelCounters)> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .upserts
            .clone()
    }
}

impl ShadowTables for FunnelRecorder {
    type Error = Scripted;

    fn find_active_jobs(
        &self,
        _now: DateTime<Utc>,
    ) -> ShadowFuture<'_, Vec<ShadowJobRow>, Self::Error> {
        Box::pin(async { Err(Scripted) })
    }

    fn attempt_stats<'database>(
        &'database self,
        _job_ids: &'database [String],
    ) -> ShadowFuture<'database, Vec<JobAttemptStats>, Self::Error> {
        Box::pin(async { Err(Scripted) })
    }

    fn insert_attempt<'database>(
        &'database self,
        _attempt: &'database ShadowAttemptRow,
    ) -> ShadowFuture<'database, (), Self::Error> {
        Box::pin(async { Err(Scripted) })
    }

    fn upsert_funnel<'database>(
        &'database self,
        job_id: &'database str,
        counters: FunnelCounters,
    ) -> ShadowFuture<'database, (), Self::Error> {
        Box::pin(async move {
            let (failed, interleaved) = {
                let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                let failure = state.fail_for.iter().any(|failed| failed == job_id);
                state.upserts.push((job_id.to_owned(), counters));
                (failure, state.interleave.take())
            };
            if let Some((late_job, queue)) = interleaved {
                queue.record(&late_job, FunnelStage::Withheld);
            }
            if failed {
                return Err(Scripted);
            }
            Ok(())
        })
    }
}

#[test]
fn shadow_eval_funnel_counts_pending_events_per_stage() {
    let queue = FunnelQueue::default();
    queue.record("job-a", FunnelStage::NotSampled);
    queue.record("job-a", FunnelStage::NotSampled);
    queue.record("job-a", FunnelStage::Unjudgeable);
    queue.record("job-b", FunnelStage::Shed);
    queue.record("job-b", FunnelStage::Withheld);
    assert_eq!(queue.pending_events(), 5);
    assert_eq!(queue.pending_events(), 5);
    let mut counters = FunnelCounters::default();
    counters.record(FunnelStage::Shed);
    counters.record(FunnelStage::Withheld);
    assert_eq!(counters.total(), 2);
}

#[tokio::test]
async fn shadow_eval_funnel_flush_upserts_each_job_and_clears_the_queue() {
    let recorder = FunnelRecorder::default();
    let queue = FunnelQueue::default();
    queue.record("job-a", FunnelStage::NotSampled);
    queue.record("job-a", FunnelStage::Unjudgeable);
    queue.record("job-b", FunnelStage::Withheld);
    queue.flush(&recorder).await;
    assert_eq!(queue.pending_events(), 0);
    assert_eq!(
        recorder.upserts(),
        vec![
            (
                "job-a".to_owned(),
                FunnelCounters {
                    not_sampled: 1,
                    unjudgeable: 1,
                    shed: 0,
                    withheld: 0,
                },
            ),
            (
                "job-b".to_owned(),
                FunnelCounters {
                    not_sampled: 0,
                    unjudgeable: 0,
                    shed: 0,
                    withheld: 1,
                },
            ),
        ]
    );
    queue.flush(&recorder).await;
    assert_eq!(recorder.upserts().len(), 2);
}

#[tokio::test]
async fn shadow_eval_funnel_flush_keeps_events_recorded_during_the_flush() {
    let queue = Arc::new(FunnelQueue::default());
    let recorder = FunnelRecorder::default();
    queue.record("job-a", FunnelStage::Shed);
    recorder.interleave("job-c", &queue);
    queue.flush(&recorder).await;
    assert_eq!(queue.pending_events(), 1);
    assert_eq!(recorder.upserts().len(), 1);
    queue.flush(&recorder).await;
    assert_eq!(queue.pending_events(), 0);
    assert_eq!(
        recorder.upserts(),
        vec![
            (
                "job-a".to_owned(),
                FunnelCounters {
                    not_sampled: 0,
                    unjudgeable: 0,
                    shed: 1,
                    withheld: 0,
                },
            ),
            (
                "job-c".to_owned(),
                FunnelCounters {
                    not_sampled: 0,
                    unjudgeable: 0,
                    shed: 0,
                    withheld: 1,
                },
            ),
        ]
    );
}

#[tokio::test]
async fn shadow_eval_funnel_flush_drops_a_failed_job_and_continues() {
    let recorder = FunnelRecorder::default();
    let queue = FunnelQueue::default();
    recorder.fail_for("job-a");
    queue.record("job-a", FunnelStage::Shed);
    queue.record("job-b", FunnelStage::Shed);
    queue.flush(&recorder).await;
    assert_eq!(recorder.upserts().len(), 2);
    assert_eq!(queue.pending_events(), 0);
    queue.record("job-b", FunnelStage::Withheld);
    queue.flush(&recorder).await;
    assert_eq!(queue.pending_events(), 0);
    let upserts = recorder.upserts();
    assert_eq!(upserts.len(), 3);
    let last = upserts.last().unwrap();
    assert_eq!(last.0, "job-b");
    assert_eq!(last.1.withheld, 1);
    assert_eq!(last.1.shed, 0);
}
