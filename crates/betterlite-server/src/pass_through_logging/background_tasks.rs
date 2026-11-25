use std::future::Future;

use tokio::task::JoinHandle;
use tokio_util::task::TaskTracker;

// A task nobody holds is lost at shutdown, so every pass-through background write runs here and the server drains it.
#[derive(Clone, Default)]
pub struct BackgroundTasks {
    // The server owns one tracker for its lifetime, so a write outlives the client that started it.
    pub tasks: TaskTracker,
}

impl BackgroundTasks {
    pub fn spawn<Work>(&self, work: Work, warning: String) -> JoinHandle<()>
    where
        Work: Future<Output = ()> + Send + 'static,
    {
        // Armed before the task exists, so a cancellation before its first poll still names the consequence.
        let cancelled = CancellationWarning {
            warning,
            armed: true,
        };
        self.tasks.spawn(run_and_disarm(work, cancelled))
    }

    pub async fn shutdown(&self) {
        self.tasks.close();
        self.tasks.wait().await;
    }
}

// The work and its guard travel into the task together, so an abort before the first poll still drops the guard.
async fn run_and_disarm<Work>(work: Work, mut cancelled: CancellationWarning)
where
    Work: Future<Output = ()>,
{
    work.await;
    cancelled.armed = false;
}

// A cancelled task skips its own finish, so the consequence it never reached is named here.
struct CancellationWarning {
    warning: String,
    armed: bool,
}

impl Drop for CancellationWarning {
    fn drop(&mut self) {
        if self.armed {
            let warning = self.warning.as_str();
            tracing::warn!("{warning}");
        }
    }
}
