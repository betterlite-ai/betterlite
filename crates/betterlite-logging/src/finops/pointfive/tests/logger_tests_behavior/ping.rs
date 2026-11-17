use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::pointfive_params::PointFiveInitParams;
use tokio::task::{JoinSet, yield_now};
use tokio_util::sync::CancellationToken;

use super::support::{FakeHttp, PRESIGNED, Reply, client, core, payload, target_body, wait_until};
use crate::finops::pointfive::logger::flush::periodic_flush;

const HOUR: Duration = Duration::from_secs(3600);

#[tokio::test]
async fn the_first_cycle_pings_an_idle_queue() {
    let http = FakeHttp::scripted(vec![Reply::answer(200)]);
    let core = core(PointFiveInitParams::default(), client(&http, 1));
    let shutdown = CancellationToken::new();
    let token = shutdown.clone();
    let mut tasks = JoinSet::new();
    tasks.spawn(async move { periodic_flush(&core, HOUR, token).await });
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
    tasks.spawn(async move { periodic_flush(&background, HOUR, token).await });
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
async fn a_later_cycle_flushes_what_arrived_since() {
    let http = FakeHttp::scripted(vec![
        Reply::with_body(200, &target_body(PRESIGNED, "objects/0")),
        Reply::answer(200),
        Reply::with_body(200, &target_body(&format!("{PRESIGNED}/1"), "objects/1")),
        Reply::answer(200),
    ]);
    let core = core(PointFiveInitParams::default(), client(&http, 3));
    core.queue.push(payload("evt-1"));
    let background = Arc::clone(&core);
    let shutdown = CancellationToken::new();
    let token = shutdown.clone();
    let mut tasks = JoinSet::new();
    tasks.spawn(async move { periodic_flush(&background, Duration::from_millis(20), token).await });
    wait_until(|| http.uploads().len() == 1).await;
    core.queue.push(payload("evt-2"));
    wait_until(|| http.uploads().len() == 2).await;
    shutdown.cancel();
    tasks
        .join_next()
        .await
        .expect("the periodic flush returns")
        .expect("the flush task ends");
    assert!(core.queue.is_empty());
}

#[tokio::test]
async fn a_failed_ping_leaves_the_loop_waiting() {
    let http = FakeHttp::scripted(vec![Reply::answer(401)]);
    let core = core(PointFiveInitParams::default(), client(&http, 1));
    let shutdown = CancellationToken::new();
    let token = shutdown.clone();
    let mut tasks = JoinSet::new();
    tasks.spawn(async move { periodic_flush(&core, HOUR, token).await });
    wait_until(|| !http.pings().is_empty()).await;
    yield_now().await;
    assert!(!tasks.is_empty());
    shutdown.cancel();
    tasks
        .join_next()
        .await
        .expect("the periodic flush returns")
        .expect("the flush task ends");
}
