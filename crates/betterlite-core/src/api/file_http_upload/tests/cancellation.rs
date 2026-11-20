use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use http::HeaderMap;

use crate::api::api_boundaries::ApiLimits;
use crate::api::file_http_upload::config::UploadParams;
use crate::api::file_http_upload::plan::{MediaPlan, UploadPlan};
use crate::api::file_http_upload::service::{
    ClientSource, LogContext, Mode, UploadBody, UploadCall, UploadDeps, UploadRequest,
    UploadTransport, prepare, send,
};
use crate::api::vector_store_http_write::error::ClientError;

use super::{API_BASE, BOUNDARY, Harness, create_call, params, pre_post_env};

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
    async fn send(&self, request: UploadRequest) -> Result<UpstreamResponse, TransportError> {
        let UploadBody::Blocks(blocks) = request.body else {
            panic!("a media upload sends blocks");
        };
        drop(blocks);
        Ok(UpstreamResponse {
            status: 200,
            headers: HeaderMap::new(),
            body: b"{}".to_vec(),
        })
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
            dropped: dropped.clone(),
        }),
        content_type: None,
    }));
    let mut harness = Harness::new(plan, API_BASE, Vec::new());
    let params = params();
    let call = create_call();
    let upload_params = UploadParams {
        api_base: Some(API_BASE),
        api_key: None,
        ssl_verify: None,
        litellm_params: &params,
    };
    let upload_call = UploadCall {
        data: &call,
        params: &upload_params,
    };
    let boundaries = || String::from(BOUNDARY);
    let source = Source;
    let deps = UploadDeps {
        config: &harness.config,
        source: &source,
        supplied: None,
        tasks: &harness.tasks,
        boundaries: &boundaries,
        limits: ApiLimits::default(),
    };
    let env = pre_post_env(&harness.redactor, &harness.sink);
    let mut log = LogContext {
        state: &mut harness.state,
        env: &env,
    };
    let prepared = prepare(&deps, Mode::Sync, &upload_call, &mut log).expect("the media plan");
    send(prepared).await.expect("the dropped body is no error");
    let stopped = tokio::time::timeout(Duration::from_secs(5), async {
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
