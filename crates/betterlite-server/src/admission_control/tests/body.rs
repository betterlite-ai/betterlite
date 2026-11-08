use std::io::Error as IoError;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::body::Bytes;
use futures_util::StreamExt as _;
use futures_util::stream;

use crate::admission_control::body::GuardedBody;

struct Guard(Arc<AtomicBool>);

impl Drop for Guard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn guarded_body_releases_at_the_final_frame() {
    let released = Arc::new(AtomicBool::new(false));
    let source = stream::iter([Ok::<Bytes, IoError>(Bytes::from_static(b"last"))]);
    let mut body = GuardedBody {
        inner: source,
        guard: Some(Guard(Arc::clone(&released))),
    };
    assert_eq!(
        body.next().await.unwrap().unwrap(),
        Bytes::from_static(b"last")
    );
    assert!(!released.load(Ordering::SeqCst));
    assert!(body.next().await.is_none());
    assert!(released.load(Ordering::SeqCst));
    drop(body);
}
