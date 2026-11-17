use std::future::Future;

use tokio::task::JoinHandle;
use tokio_util::task::TaskTracker;

// LiteLLM holds every billing poller in a module set until it finishes, since a weakly referenced task can vanish mid-poll.
#[derive(Default)]
pub struct BillingTasks {
    // The server owns one tracker for its lifetime, so a poller outlives the client that started it.
    pub tasks: TaskTracker,
}

impl BillingTasks {
    pub fn spawn<Billing>(&self, billing: Billing) -> JoinHandle<()>
    where
        Billing: Future<Output = ()> + Send + 'static,
    {
        self.tasks.spawn(async move {
            let mut warning = CancellationWarning { armed: true };
            billing.await;
            warning.armed = false;
        })
    }

    pub async fn shutdown(&self) {
        self.tasks.close();
        self.tasks.wait().await;
    }
}

// A cancelled poller skips its own error handling, so the charge it never wrote is named here.
struct CancellationWarning {
    armed: bool,
}

impl Drop for CancellationWarning {
    fn drop(&mut self) {
        if self.armed {
            tracing::warn!(
                "TinyFish passthrough: billing poller cancelled mid-poll; the run may go unbilled"
            );
        }
    }
}
