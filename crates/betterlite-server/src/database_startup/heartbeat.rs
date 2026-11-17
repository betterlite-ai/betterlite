use std::sync::Arc;
use std::time::Duration;

use betterlite_db::worker_heartbeat::{
    HEARTBEAT_INTERVAL_SECONDS, HeartbeatDatabase, ProxyWorkerHeartbeat,
};
use futures_util::future::{Either, select};
use tokio::sync::Notify;
use tokio::task::{JoinError, JoinSet};
use tokio::time::sleep;

// a stopped worker deregisters so a departed pod stops counting toward the live-worker census
pub struct HeartbeatTask {
    stop: Arc<Notify>,
    tasks: JoinSet<()>,
}

impl HeartbeatTask {
    #[must_use]
    pub async fn start<Database>(
        heartbeat: Arc<ProxyWorkerHeartbeat>,
        database: Arc<Database>,
    ) -> Self
    where
        Database: HeartbeatDatabase + Send + Sync + 'static,
    {
        heartbeat.beat(database.as_ref()).await;
        let stop = Arc::new(Notify::new());
        let mut tasks = JoinSet::new();
        tasks.spawn(run(heartbeat, database, Arc::clone(&stop)));
        Self { stop, tasks }
    }

    pub async fn stop(self) -> Result<(), JoinError> {
        self.stop.notify_one();
        let mut tasks = self.tasks;
        while let Some(outcome) = tasks.join_next().await {
            outcome?;
        }
        Ok(())
    }
}

async fn run<Database>(
    heartbeat: Arc<ProxyWorkerHeartbeat>,
    database: Arc<Database>,
    stop: Arc<Notify>,
) where
    Database: HeartbeatDatabase + Send + Sync + 'static,
{
    loop {
        let tick = Box::pin(sleep(Duration::from_secs(u64::from(
            HEARTBEAT_INTERVAL_SECONDS,
        ))));
        let stopping = Box::pin(stop.notified());
        match select(tick, stopping).await {
            Either::Left(_) => heartbeat.beat(database.as_ref()).await,
            Either::Right(_) => break,
        }
    }
    heartbeat.deregister(database.as_ref()).await;
}
