use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use super::super::super::recording::{RecordingBody, ResponseBody as _};
use super::{FakeBody, FakeError};

type Log = Arc<Mutex<Vec<Arc<FakeError>>>>;

fn recorded_into(log: &Log, body: FakeBody) -> RecordingBody<FakeBody> {
    let shared = Arc::clone(log);
    RecordingBody::new(
        body,
        Arc::new(move |error: &Arc<FakeError>| shared.lock().unwrap().push(Arc::clone(error))),
    )
}

#[tokio::test]
async fn forwards_chunks_without_recording() {
    let log = Log::default();
    let mut body = recorded_into(
        &log,
        FakeBody::new(vec![Ok(b"a".to_vec()), Ok(b"b".to_vec())]),
    );
    assert_eq!(body.next_chunk().await.unwrap().unwrap(), b"a");
    assert_eq!(body.next_chunk().await.unwrap().unwrap(), b"b");
    assert!(body.next_chunk().await.is_none());
    assert!(log.lock().unwrap().is_empty());
}

#[tokio::test]
async fn records_an_iteration_failure_and_returns_the_same_error() {
    let log = Log::default();
    let mut body = recorded_into(
        &log,
        FakeBody::new(vec![Ok(b"a".to_vec()), Err(FakeError::Boom("read"))]),
    );
    body.next_chunk().await.unwrap().unwrap();
    let error = body.next_chunk().await.unwrap().unwrap_err();
    assert_eq!(log.lock().unwrap().len(), 1);
    assert!(Arc::ptr_eq(&log.lock().unwrap()[0], &error));
    assert_eq!(*error, FakeError::Boom("read"));
}

#[tokio::test]
async fn forwards_close_and_records_its_failure() {
    let log = Log::default();
    let mut inner = FakeBody::new(vec![]);
    inner.close_error = Some(FakeError::Boom("close"));
    let closed = Arc::clone(&inner.closed);
    let mut body = recorded_into(&log, inner);
    let error = body.close().await.unwrap_err();
    assert!(closed.load(Ordering::SeqCst));
    assert!(Arc::ptr_eq(&log.lock().unwrap()[0], &error));
}

#[tokio::test]
async fn a_clean_close_records_nothing() {
    let log = Log::default();
    let mut body = recorded_into(&log, FakeBody::new(vec![]));
    body.close().await.unwrap();
    assert!(log.lock().unwrap().is_empty());
}

#[tokio::test]
async fn records_a_failure_while_reading_everything() {
    let log = Log::default();
    let mut body = recorded_into(&log, FakeBody::new(vec![Err(FakeError::Boom("read"))]));
    let error = body.read_all().await.unwrap_err();
    assert!(Arc::ptr_eq(&log.lock().unwrap()[0], &error));
}
