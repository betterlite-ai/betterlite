#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;
use tokio::task::{JoinSet, yield_now};
use tokio::time::advance;
use tokio_util::sync::CancellationToken;

use super::{SpendMonitorSettings, monitor_spend_logs_queue, wait_for_flush_request};
use crate::spend_flush::fakes::tests::{Events, FakeService};
use crate::spend_log_storage::rows::SpendLogQueue;

// A running monitor over a fake service and a queue nobody fills; the census reads the test's script on every pass.
struct Monitor {
    events: Arc<Events>,
    service: Arc<FakeService>,
    queue: Arc<SpendLogQueue>,
    shutdown: CancellationToken,
    tasks: JoinSet<()>,
}

impl Monitor {
    fn start(census: Vec<i64>, settings: SpendMonitorSettings) -> Self {
        let events = Arc::new(Events::default());
        let service = Arc::new(FakeService::new(Arc::clone(&events), census));
        let queue = Arc::new(SpendLogQueue::new(usize::MAX));
        let shutdown = CancellationToken::new();
        let mut tasks = JoinSet::new();
        tasks.spawn(monitor_spend_logs_queue(
            Arc::clone(&service),
            Arc::clone(&queue),
            settings,
            shutdown.clone(),
        ));
        Self {
            events,
            service,
            queue,
            shutdown,
            tasks,
        }
    }

    async fn stop(&mut self) {
        self.shutdown.cancel();
        while self.tasks.join_next().await.is_some() {}
    }

    // Yields, never advances, until the census appears: the monitor reads it on the poll after its wait ends.
    async fn reach(&self, censuses: usize) {
        for _ in 0..64_usize {
            if self.service.census_times().len() >= censuses {
                return;
            }
            yield_now().await;
        }
        assert!(self.service.census_times().len() >= censuses);
    }

    // The paused clock's readings of the census, one gap per wait between two passes.
    fn gaps(&self) -> Vec<Duration> {
        self.service
            .census_times()
            .windows(2)
            .map(|pair| pair[1].duration_since(pair[0]))
            .collect()
    }

    fn update_count(&self) -> usize {
        self.events
            .entries()
            .iter()
            .filter(|entry| entry.starts_with("update "))
            .count()
    }
}

const fn settings(queue_size_threshold: i64, base_interval: Duration) -> SpendMonitorSettings {
    SpendMonitorSettings {
        queue_size_threshold,
        base_interval,
    }
}

#[tokio::test(start_paused = true)]
async fn a_full_queue_waits_its_base_interval_every_pass() {
    let mut monitor = Monitor::start(vec![100], settings(100, Duration::from_secs(2)));
    monitor.reach(1).await;
    advance(Duration::from_millis(1999)).await;
    assert_eq!(monitor.service.census_times().len(), 1);
    advance(Duration::from_millis(1)).await;
    monitor.reach(2).await;
    advance(Duration::from_secs(2)).await;
    monitor.reach(3).await;
    monitor.stop().await;
    assert_eq!(
        monitor.gaps(),
        [Duration::from_secs(2), Duration::from_secs(2)]
    );
}

#[tokio::test(start_paused = true)]
async fn a_below_threshold_queue_grows_the_wait_by_half_up_to_the_cap() {
    let mut monitor = Monitor::start(vec![1], settings(100, Duration::from_secs(8)));
    monitor.reach(1).await;
    advance(Duration::from_secs(12)).await;
    monitor.reach(2).await;
    advance(Duration::from_secs(18)).await;
    monitor.reach(3).await;
    advance(Duration::from_secs(27)).await;
    monitor.reach(4).await;
    advance(Duration::from_secs(30)).await;
    monitor.reach(5).await;
    advance(Duration::from_secs(30)).await;
    monitor.reach(6).await;
    monitor.stop().await;
    let expected = [12, 18, 27, 30, 30].map(Duration::from_secs);
    assert_eq!(monitor.gaps(), expected);
    assert_eq!(monitor.update_count(), 6);
}

#[tokio::test(start_paused = true)]
async fn an_empty_queue_backs_off_without_running_a_pass() {
    let mut monitor = Monitor::start(vec![0], settings(100, Duration::from_secs(2)));
    monitor.reach(1).await;
    advance(Duration::from_secs(3)).await;
    monitor.reach(2).await;
    monitor.stop().await;
    assert_eq!(monitor.gaps(), [Duration::from_secs(3)]);
    assert_eq!(
        monitor.events.entries(),
        ["recover", "census 0", "recover", "census 0"]
    );
}

#[tokio::test(start_paused = true)]
async fn a_flush_request_wakes_the_wait_and_resets_the_backoff() {
    let mut monitor = Monitor::start(vec![1], settings(100, Duration::from_secs(2)));
    monitor.reach(1).await;
    advance(Duration::from_secs(1)).await;
    monitor.queue.request_flush();
    monitor.reach(2).await;
    advance(Duration::from_secs(3)).await;
    monitor.reach(3).await;
    monitor.stop().await;
    assert_eq!(
        monitor.gaps(),
        [Duration::from_secs(1), Duration::from_secs(3)]
    );
}

#[tokio::test(start_paused = true)]
async fn a_failed_pass_sleeps_the_grown_interval() {
    let mut monitor = Monitor::start(vec![100], settings(100, Duration::from_secs(2)));
    monitor.service.fail_updates();
    monitor.reach(1).await;
    advance(Duration::from_secs(3)).await;
    monitor.reach(2).await;
    advance(Duration::from_secs(3)).await;
    monitor.reach(3).await;
    monitor.stop().await;
    assert_eq!(
        monitor.gaps(),
        [Duration::from_secs(3), Duration::from_secs(3)]
    );
    assert_eq!(monitor.update_count(), 3);
}

#[tokio::test]
async fn a_pending_request_wakes_the_wait_early() {
    let signal = Notify::new();
    signal.notify_one();
    assert!(wait_for_flush_request(&signal, Duration::from_mins(1)).await);
}

#[tokio::test(start_paused = true)]
async fn an_unrequested_wait_times_out() {
    let signalled = wait_for_flush_request(&Notify::new(), Duration::from_secs(2)).await;
    assert!(!signalled);
}

#[tokio::test]
async fn a_zero_interval_wait_leaves_a_pending_request_for_the_next_one() {
    let signal = Notify::new();
    signal.notify_one();
    assert!(!wait_for_flush_request(&signal, Duration::ZERO).await);
    assert!(wait_for_flush_request(&signal, Duration::from_mins(1)).await);
}
