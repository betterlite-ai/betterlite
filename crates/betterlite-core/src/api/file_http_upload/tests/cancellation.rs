use std::future::{Future, ready};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use http::HeaderMap;
use tokio::time::timeout;

use crate::api::file_http_upload::plan::{MediaPlan, UploadPlan};
use crate::api::file_http_upload::service::{Mode, UploadBody, UploadRequest, UploadTransport};
use crate::api::vector_store_http_write::error::ClientError;
use crate::api::vector_store_http_write::service::ClientSource;

use super::{API_BASE, Harness, RunCall, create_call, params};

// An endless media body that records when the blocking producer lets it go
struct Endless {
    dropped: Arc<AtomicBool>,
}

impl Iterator for Endless {
    type Item = Vec<u8>;

    fn next(&mut self) -> Option<Vec<u8>> {
        Some(vec![0_u8; 1024])
    }
}

impl Drop for Endless {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

// A transport that drops the body instead of reading it, as a cancelled request does
struct Dropper;

impl UploadTransport for Dropper {
    fn send(
        &self,
        request: UploadRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> {
        let UploadBody::Blocks(blocks) = request.body else {
            panic!("a media upload sends blocks");
        };
        drop(blocks);
        ready(Ok(UpstreamResponse {
            status: 200,
            headers: HeaderMap::new(),
            body: b"{}".to_vec(),
        }))
    }
}

struct Source;

impl ClientSource for Source {
    type Client = Dropper;

    fn client(&self, _ssl_verify: Option<bool>) -> Result<Dropper, ClientError> {
        Ok(Dropper)
    }
}

// Dropping the body stream closes the block channel, so the blocking producer stops and frees the input
#[tokio::test]
async fn media_cancellation_closes_the_block_producer() {
    let dropped = Arc::new(AtomicBool::new(false));
    let plan = UploadPlan::Media(Box::new(MediaPlan {
        body_stream: Box::new(Endless {
            dropped: Arc::clone(&dropped),
        }),
        content_type: None,
    }));
    let mut harness = Harness::new(plan, API_BASE, Vec::new());
    let run_call = RunCall {
        mode: Mode::Sync,
        call: &create_call(),
        params: &params(),
    };
    harness
        .run_against(&Source, None, run_call)
        .await
        .expect("the dropped body is no error");
    let stopped = timeout(Duration::from_secs(5), async {
        harness.tasks.close();
        harness.tasks.wait().await;
    })
    .await;
    assert!(
        stopped.is_ok(),
        "the blocking producer exits once the receiver drops"
    );
    assert!(dropped.load(Ordering::SeqCst));
}
