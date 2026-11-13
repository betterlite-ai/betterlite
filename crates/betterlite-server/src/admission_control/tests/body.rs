use std::io::Error as IoError;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::body::{Body, Bytes};
use futures_util::StreamExt as _;
use futures_util::stream;
use http_body::Body as _;

use crate::admission_control::body::GuardedBody;

struct Guard(Arc<AtomicBool>);

impl Drop for Guard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[test]
fn guarded_body_reports_the_end_of_an_empty_body() {
    let released = Arc::new(AtomicBool::new(false));
    let empty = GuardedBody {
        inner: Body::empty(),
        guard: Some(Guard(Arc::clone(&released))),
    };
    let pending = GuardedBody {
        inner: Body::from("data"),
        guard: Some(Guard(released)),
    };
    assert!(empty.is_end_stream());
    assert!(!pending.is_end_stream());
}

#[tokio::test]
async fn guarded_body_releases_at_the_final_frame() {
    let released = Arc::new(AtomicBool::new(false));
    let source = stream::iter([Ok::<Bytes, IoError>(Bytes::from_static(b"last"))]);
    let mut body = Body::new(GuardedBody {
        inner: Body::from_stream(source),
        guard: Some(Guard(Arc::clone(&released))),
    })
    .into_data_stream();
    assert_eq!(
        body.next().await.unwrap().unwrap(),
        Bytes::from_static(b"last")
    );
    assert!(!released.load(Ordering::SeqCst));
    assert!(body.next().await.is_none());
    assert!(released.load(Ordering::SeqCst));
    drop(body);
}

#[tokio::test]
async fn guarded_body_dropped_mid_stream_releases_the_guard() {
    let released = Arc::new(AtomicBool::new(false));
    let source = stream::iter([
        Ok::<Bytes, IoError>(Bytes::from_static(b"first")),
        Ok::<Bytes, IoError>(Bytes::from_static(b"last")),
    ]);
    let mut body = Body::new(GuardedBody {
        inner: Body::from_stream(source),
        guard: Some(Guard(Arc::clone(&released))),
    })
    .into_data_stream();
    assert_eq!(
        body.next().await.unwrap().unwrap(),
        Bytes::from_static(b"first")
    );
    assert!(!released.load(Ordering::SeqCst));
    drop(body);
    assert!(released.load(Ordering::SeqCst));
}
