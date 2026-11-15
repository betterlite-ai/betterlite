use std::sync::Mutex;

use tokio_util::task::TaskTracker;

use super::support::capture_logs;
use crate::db_lifecycle::TrackedTask;

#[tokio::test]
async fn a_task_that_panics_is_reported_when_it_stops() {
    let (logs, guard) = capture_logs();
    let tracked = TrackedTask {
        handle: Mutex::new(None),
    };
    let tasks = TaskTracker::new();
    assert!(tracked.start_with(&tasks, async {
        panic!("the loop died");
    }));
    tasks.wait().await;
    assert!(tracked.stop().await);
    drop(guard);
    assert!(
        logs.lines()
            .iter()
            .any(|line| line.contains("a database lifecycle task ended with an error")),
        "{:?}",
        logs.lines()
    );
}
