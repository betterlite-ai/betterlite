use std::sync::PoisonError;
use std::thread;
use std::time::{Duration, Instant};

use tokio::task::JoinSet;
use tokio::time::timeout as deadline_timeout;

use super::super::gate::InterruptGate;

// The source polls the interrupted thread for this long, and this often, before it calls the thread stuck.
pub const INTERRUPT_GRACE: Duration = Duration::from_secs(1);
pub const INTERRUPT_POLL: Duration = Duration::from_millis(50);

// Keeps aiming interrupts at a worker that outlived its deadline; a thread stuck in a native call keeps running.
pub fn interrupt_worker<F>(gate: &InterruptGate, mut finished: F, label: &str)
where
    F: FnMut() -> bool,
{
    let bound = Instant::now().checked_add(INTERRUPT_GRACE);
    while gate.interrupt() && bound.is_none_or(|limit| Instant::now() < limit) {
        thread::park_timeout(INTERRUPT_POLL);
        if finished() {
            return;
        }
    }
    if gate
        .thread
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .is_some()
    {
        tracing::error!(
            "guardrail-code:{label} is still running after its timeout; it is stuck in a call that cannot be interrupted"
        );
    }
}

// Aborts the awaited task every poll interval until it stops, the event-loop counterpart of interrupting a thread.
pub async fn cancel_task<T>(tasks: &mut JoinSet<T>, label: &str)
where
    T: 'static,
{
    let bound = Instant::now().checked_add(INTERRUPT_GRACE);
    let mut running = true;
    while running && bound.is_none_or(|limit| Instant::now() < limit) {
        tasks.abort_all();
        running = deadline_timeout(INTERRUPT_POLL, tasks.join_next())
            .await
            .is_err();
    }
    if running {
        tracing::error!(
            "guardrail-code:{label} is still running after its timeout; it keeps swallowing cancellation"
        );
    }
}
