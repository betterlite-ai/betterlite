use std::sync::Arc;
use std::time::Duration;

use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

use super::coordinator::SyncCoordinator;
use super::status::StatusReporter;

pub(crate) const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);

pub(crate) struct Heartbeat {
    pub(crate) coordinator: Arc<dyn SyncCoordinator>,
    pub(crate) owner: String,
    pub(crate) status: StatusReporter,
    pub(crate) run: CancellationToken,
    pub(crate) stop: CancellationToken,
}

impl Heartbeat {
    pub(crate) async fn run(self) {
        loop {
            tokio::select! {
                () = self.stop.cancelled() => return,
                () = sleep(HEARTBEAT_INTERVAL) => {
                    let status = self.status.snapshot();
                    let renewed = tokio::select! {
                        () = self.stop.cancelled() => return,
                        renewed = self.coordinator.heartbeat(&self.owner, &status) => renewed,
                    };
                    if !renewed {
                        self.run.cancel();
                        return;
                    }
                }
            }
        }
    }
}
