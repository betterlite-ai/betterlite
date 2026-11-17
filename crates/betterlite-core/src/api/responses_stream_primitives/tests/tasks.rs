use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::{Notify, oneshot};
use tokio_util::task::TaskTracker;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context as LayerContext, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use crate::api::responses_stream_primitives::tasks::spawn_logging_task;

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<(Level, String)>>>);

impl Capture {
    fn events(&self) -> Vec<(Level, String)> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Default)]
struct MessageVisitor(String);

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0 = value.to_owned();
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Capture {
    fn on_event(&self, event: &Event<'_>, _context: LayerContext<'_, Inner>) {
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((*event.metadata().level(), visitor.0));
    }
}

fn capture() -> (Capture, DefaultGuard) {
    let captured = Capture::default();
    let guard = set_default(registry().with(captured.clone()));
    (captured, guard)
}

#[test]
fn spawning_without_a_runtime_returns_null() {
    let pending = TaskTracker::new();
    let spawned = spawn_logging_task(&pending, "probe", async { Ok::<(), &str>(()) });
    assert!(spawned.is_none());
    assert!(pending.is_empty());
}

#[tokio::test]
async fn a_failing_task_logs_its_name_and_error() {
    let (captured, _guard) = capture();
    let pending = TaskTracker::new();
    let spawned = spawn_logging_task(&pending, "probe", async { Err::<(), &str>("broken") });
    assert!(spawned.is_some());
    pending.close();
    pending.wait().await;
    let events = captured.events();
    let Some((level, message)) = events.first() else {
        panic!("the failure is logged");
    };
    assert_eq!(*level, Level::ERROR);
    assert_eq!(message, "probe failed: broken");
    assert_eq!(events.len(), 1);
}

#[tokio::test]
async fn a_successful_task_logs_nothing() {
    let (captured, _guard) = capture();
    let pending = TaskTracker::new();
    let spawned = spawn_logging_task(&pending, "probe", async { Ok::<(), &str>(()) });
    assert!(spawned.is_some());
    pending.close();
    pending.wait().await;
    assert!(captured.events().is_empty());
}

#[tokio::test]
async fn spawned_work_outlives_its_handle_and_leaves_the_pending_set() {
    let pending = TaskTracker::new();
    let release = Arc::new(Notify::new());
    let (done, completed) = oneshot::channel();
    let held = Arc::clone(&release);
    let spawned = spawn_logging_task(&pending, "probe", async move {
        held.notified().await;
        let _ = done.send(());
        Ok::<(), &str>(())
    });
    assert!(spawned.is_some());
    drop(spawned);
    assert_eq!(pending.len(), 1);
    release.notify_one();
    completed.await.expect("the task runs to the end");
    pending.close();
    pending.wait().await;
    assert!(pending.is_empty());
}

#[tokio::test]
async fn a_cancelled_task_logs_nothing() {
    let (captured, _guard) = capture();
    let pending = TaskTracker::new();
    let spawned = spawn_logging_task(
        &pending,
        "probe",
        std::future::pending::<Result<(), &str>>(),
    );
    let Some(handle) = spawned else {
        panic!("the runtime is running");
    };
    handle.abort();
    pending.close();
    pending.wait().await;
    assert!(captured.events().is_empty());
}
