use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_azure::image_transport::request::{ImageClientConstructor, TimeoutPhases};
use betterlite_azure::image_transport::{ImageReply, ImageRequest, ImageTransport};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::image_generation::logging::{
    ImageGenerationLogger, PostCallRecord, PreCallRecord,
};
use http::header::HeaderMap;
use serde_json::Value;

pub(crate) struct FixtureState {
    pub(crate) calls: Mutex<Vec<RecordedCall>>,
    pub(crate) reply: Mutex<String>,
}

#[derive(Clone)]
pub(crate) struct RecordedCall {
    pub(crate) url: String,
    pub(crate) body: Option<Value>,
    // only the auth headers travel back, since httpx adds its own defaults
    pub(crate) headers: BTreeMap<String, Option<String>>,
}

pub(crate) struct FixtureTransport {
    pub(crate) state: Arc<FixtureState>,
}

impl ImageTransport for FixtureTransport {
    fn send(
        &self,
        request: ImageRequest,
    ) -> impl Future<
        Output = Result<ImageReply, betterlite_http::proxy::transport_error::TransportError>,
    > + Send {
        let mut headers = BTreeMap::new();
        for name in ["api-key", "authorization"] {
            let value = request
                .headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            headers.insert(name.to_owned(), value);
        }
        self.state
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedCall {
                url: request.url.clone(),
                body: serde_json::from_slice(&request.body).ok(),
                headers,
            });
        let body = self
            .state
            .reply
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        ready(Ok(ImageReply {
            status: 200,
            headers: HeaderMap::new(),
            body: body.into_bytes(),
        }))
    }
}

impl ImageClientConstructor for FixtureTransport {
    type Client = Self;

    fn construct(&self, _timeout: TimeoutPhases) -> Self {
        Self {
            state: Arc::clone(&self.state),
        }
    }
}

#[derive(Default)]
pub(crate) struct RecordingLogger {
    pub(crate) pre: Vec<PreCallRecord>,
    pub(crate) post: Vec<PostCallRecord>,
}

impl ImageGenerationLogger for RecordingLogger {
    fn pre_call(&mut self, record: PreCallRecord) -> Result<(), PrePostError> {
        self.pre.push(record);
        Ok(())
    }

    fn post_call(&mut self, record: PostCallRecord) {
        self.post.push(record);
    }
}
