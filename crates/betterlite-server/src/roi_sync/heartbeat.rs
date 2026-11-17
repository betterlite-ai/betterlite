use std::pin::pin;
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::{Either, select};
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
            if let Either::Left(((), _wait)) =
                select(pin!(self.stop.cancelled()), pin!(sleep(HEARTBEAT_INTERVAL))).await
            {
                return;
            }
            let status = self.status.snapshot();
            let renewed = match select(
                pin!(self.stop.cancelled()),
                pin!(self.coordinator.heartbeat(&self.owner, &status)),
            )
            .await
            {
                Either::Left(((), _heartbeat)) => return,
                Either::Right((renewed, _stop)) => renewed,
            };
            if !renewed {
                self.run.cancel();
                return;
            }
        }
    }
}
