use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_config::sections::pointfive_params::PointFiveInitParams;
use betterlite_http::proxy::transport_error::TransportError;
use tokio::sync::Semaphore;
use tokio::task::{JoinSet, yield_now};

use super::support::{
    FakeHttp, FakeUrlCheck, PRESIGNED, RecordingSleeper, Reply, Seen, client, client_over, core,
    logged, payload, target_body, wait_until,
};
use crate::finops::pointfive::logger::flush::flush_queue;
use crate::finops::pointfive::upload::transport::{UploadHttp, UploadRequest, UploadResponse};

// a transport that records each request and holds it until the test opens the gate
struct GatedHttp {
    gate: Arc<Semaphore>,
    replies: Arc<Mutex<VecDeque<Reply>>>,
    requests: Arc<Mutex<Vec<Seen>>>,
}

impl GatedHttp {
    fn scripted(replies: Vec<Reply>) -> Self {
        Self {
            gate: Arc::new(Semaphore::new(0)),
            replies: Arc::new(Mutex::new(VecDeque::from(replies))),
            requests: Arc::default(),
        }
    }

    fn open(&self) {
        self.gate.add_permits(1);
    }

    fn requests(&self) -> usize {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

#[async_trait]
impl UploadHttp for GatedHttp {
    async fn send(&self, request: UploadRequest) -> Result<UploadResponse, TransportError> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Seen {
                method: request.method,
                url: request.url.to_string(),
                body: request.body,
            });
        // a returned permit would let every later request through without another open
        self.gate
            .acquire()
            .await
            .expect("the test keeps the gate open")
            .forget();
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .expect("the test scripted a reply");
        match reply {
            Reply::Answer { status, body } => Ok(UploadResponse { status, body }),
            Reply::Transport => Err(TransportError::BodyTooLarge { limit: 1 }),
        }
    }
}

fn upload_pair() -> [Reply; 2] {
    [
        Reply::with_body(200, &target_body(PRESIGNED, "objects/0")),
        Reply::answer(200),
    ]
}

#[tokio::test]
async fn an_empty_queue_pings_instead_of_uploading() {
    let http = FakeHttp::scripted(vec![Reply::answer(200)]);
    let core = core(PointFiveInitParams::default(), client(&http, 1));
    flush_queue(&core, true).await;
    assert_eq!(http.pings().len(), 1);
    assert!(http.uploads().is_empty());
}

#[tokio::test]
async fn an_empty_queue_pings_without_the_skip_flag() {
    let http = FakeHttp::scripted(vec![Reply::answer(200)]);
    let core = core(PointFiveInitParams::default(), client(&http, 1));
    flush_queue(&core, false).await;
    assert_eq!(http.pings().len(), 1);
    assert!(http.uploads().is_empty());
}

#[tokio::test]
async fn a_queued_batch_is_uploaded_and_the_flag_clears() {
    let pair = upload_pair();
    let http = FakeHttp::scripted(vec![pair[0].clone(), pair[1].clone()]);
    let core = core(PointFiveInitParams::default(), client(&http, 1));
    core.queue.push(payload("evt-1"));
    core.queue.push(payload("evt-2"));
    flush_queue(&core, false).await;
    assert_eq!(http.uploads().len(), 1);
    assert!(http.pings().is_empty());
    assert!(core.queue.is_empty());
    assert!(!core.flushing.load(Ordering::SeqCst));
}

#[tokio::test]
async fn skip_if_flushing_returns_before_sending() {
    let http = FakeHttp::default();
    let core = core(PointFiveInitParams::default(), client(&http, 1));
    core.flushing.store(true, Ordering::SeqCst);
    core.queue.push(payload("evt-1"));
    flush_queue(&core, true).await;
    assert!(http.requests().is_empty());
    assert_eq!(core.queue.len(), 1);
}

#[tokio::test]
async fn without_the_skip_flag_the_flush_runs_while_flushing() {
    let pair = upload_pair();
    let http = FakeHttp::scripted(vec![pair[0].clone(), pair[1].clone()]);
    let core = core(PointFiveInitParams::default(), client(&http, 1));
    core.flushing.store(true, Ordering::SeqCst);
    core.queue.push(payload("evt-1"));
    flush_queue(&core, false).await;
    assert_eq!(http.uploads().len(), 1);
    assert!(core.queue.is_empty());
}

#[tokio::test]
async fn a_failed_ping_does_not_stop_the_flush() {
    let http = FakeHttp::scripted(vec![Reply::answer(401)]);
    let core = core(PointFiveInitParams::default(), client(&http, 1));
    flush_queue(&core, true).await;
    assert_eq!(http.pings().len(), 1);
    assert!(!core.flushing.load(Ordering::SeqCst));
}

#[tokio::test]
async fn a_failed_upload_keeps_the_queued_records() {
    let http = FakeHttp::scripted(vec![Reply::answer(500)]);
    let core = core(PointFiveInitParams::default(), client(&http, 1));
    core.queue.push(payload("evt-1"));
    let ((), text) = logged(flush_queue(&core, false)).await;
    assert_eq!(core.queue.len(), 1);
    assert!(!core.flushing.load(Ordering::SeqCst));
    assert!(text.contains("the batch send failed"));
    assert!(text.contains("unsent records stay queued"));
}

#[tokio::test]
async fn a_flush_waiting_on_the_lock_keeps_the_flag_set_while_it_sends() {
    let http = Arc::new(GatedHttp::scripted(vec![
        Reply::with_body(200, &target_body(PRESIGNED, "objects/0")),
        Reply::answer(200),
        Reply::with_body(200, &target_body(PRESIGNED, "objects/1")),
        Reply::answer(200),
    ]));
    let core = core(
        PointFiveInitParams::default(),
        client_over(
            Arc::<GatedHttp>::clone(&http),
            Arc::new(FakeUrlCheck),
            Arc::new(RecordingSleeper::default()),
            1,
        ),
    );
    core.queue.push(payload("evt-1"));
    let mut tasks = JoinSet::new();
    let first = Arc::clone(&core);
    tasks.spawn(async move { flush_queue(&first, false).await });
    wait_until(|| http.requests() == 1).await;

    // the second flush queues up behind the lock the first one holds
    let second = Arc::clone(&core);
    tasks.spawn(async move { flush_queue(&second, false).await });
    yield_now().await;
    http.open();
    wait_until(|| http.requests() == 2).await;
    core.queue.push(payload("evt-2"));
    http.open();
    wait_until(|| http.requests() == 3).await;

    // the queue must refuse to trim while the queued flush is uploading
    assert!(core.flushing.load(Ordering::SeqCst));

    http.open();
    wait_until(|| http.requests() == 4).await;
    http.open();
    while tasks.join_next().await.is_some() {}
    assert!(core.queue.is_empty());
    assert!(!core.flushing.load(Ordering::SeqCst));
}
