use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::pointfive_params::PointFiveInitParams;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use super::support::{FakeHttp, Reply, client, core, payload, wait_until};

const HOUR: Duration = Duration::from_secs(3600);

#[tokio::test]
async fn the_first_cycle_pings_an_idle_queue() {
    let http = FakeHttp::scripted(vec![Reply::answer(200)]);
    let core = core(PointFiveInitParams::default(), client(&http, 1));
    let shutdown = CancellationToken::new();
    let token = shutdown.clone();
    let mut tasks = JoinSet::new();
    tasks.spawn(async move { core.periodic_flush(HOUR, token).await });
    wait_until(|| !http.pings().is_empty()).await;
    shutdown.cancel();
    tasks
        .join_next()
        .await
        .expect("the periodic flush returns")
        .expect("the flush task ends");
    assert_eq!(http.pings().len(), 1);
    assert!(http.uploads().is_empty());
}

#[tokio::test]
async fn the_first_cycle_uploads_a_queue_instead_of_pinging() {
    let http = FakeHttp::uploading(1);
    let core = core(PointFiveInitParams::default(), client(&http, 3));
    core.queue.push(payload("evt-1"));
    let background = Arc::clone(&core);
    let shutdown = CancellationToken::new();
    let token = shutdown.clone();
    let mut tasks = JoinSet::new();
    tasks.spawn(async move { background.periodic_flush(HOUR, token).await });
    wait_until(|| !http.uploads().is_empty()).await;
    shutdown.cancel();
    tasks
        .join_next()
        .await
        .expect("the periodic flush returns")
        .expect("the flush task ends");
    assert!(http.pings().is_empty());
    assert!(core.queue.is_empty());
}

#[tokio::test]
async fn a_failed_ping_leaves_the_loop_waiting() {
    let http = FakeHttp::scripted(vec![Reply::answer(401)]);
    let core = core(PointFiveInitParams::default(), client(&http, 1));
    let shutdown = CancellationToken::new();
    let token = shutdown.clone();
    let mut tasks = JoinSet::new();
    tasks.spawn(async move { core.periodic_flush(HOUR, token).await });
    wait_until(|| !http.pings().is_empty()).await;
    tokio::task::yield_now().await;
    assert!(!tasks.is_empty());
    shutdown.cancel();
    tasks
        .join_next()
        .await
        .expect("the periodic flush returns")
        .expect("the flush task ends");
}
