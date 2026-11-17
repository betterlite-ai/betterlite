use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_bedrock::agentcore_stream::transport::{
    Chunks, HandlerKind, PostRequest, StreamHandler, StreamHead,
};
use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use futures_util::{StreamExt as _, stream};
use http::HeaderMap;
use serde_json::{Value, json};

use super::support::{failure, header_map, sorted_pairs};
use super::types::{Case, Failure};
use betterlite_oracle_helper_api::error::HelperError;

pub(super) struct Fixture {
    status: u16,
    headers: HeaderMap,
    body: Vec<u8>,
    failure: Option<Failure>,
    posts: Mutex<Vec<Value>>,
}

impl Fixture {
    pub(super) fn new(case: &Case) -> Result<Self, Box<HelperError>> {
        Ok(Self {
            status: case.status,
            headers: header_map(&case.response_headers)?,
            body: hex::decode(&case.body).map_err(failure)?,
            failure: case.failure,
            posts: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn first_post(&self) -> Value {
        self.posts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .first()
            .cloned()
            .unwrap_or(Value::Null)
    }
}

#[async_trait]
impl StreamHandler for Fixture {
    fn kind(&self) -> HandlerKind {
        HandlerKind::Async
    }

    async fn post(&self, request: PostRequest) -> Result<StreamHead, TransportError> {
        self.posts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({
                "url": request.url.as_str(),
                "body": request.body,
                "headers": sorted_pairs(&request.headers),
                "stream": request.stream,
            }));
        if let Some(failure) = self.failure {
            return Err(transport_error(failure));
        }
        Ok(StreamHead {
            status: self.status,
            headers: self.headers.clone(),
            chunks: chunk_stream(&self.body),
        })
    }
}

fn transport_error(failure: Failure) -> TransportError {
    match failure {
        Failure::Timeout => TransportError::Rejected(HttpError::Timeout),
        Failure::Other => TransportError::Closed,
    }
}

// One transport read of the whole fixture body, the way a buffered HTTP client delivers it.
fn chunk_stream(body: &[u8]) -> Chunks {
    let reads: Vec<Result<Vec<u8>, HttpError>> = vec![Ok(body.to_vec())];
    stream::iter(reads).boxed()
}
