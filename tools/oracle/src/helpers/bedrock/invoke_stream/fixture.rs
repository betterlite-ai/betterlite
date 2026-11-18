use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_bedrock::agentcore_stream::transport::{
    HandlerKind, PostRequest, StreamHandler, StreamHead,
};
use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use futures_util::{StreamExt as _, stream};
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::HelperError;

use super::support::header_map;

pub(super) struct Fixture {
    kind: HandlerKind,
    headers: http::HeaderMap,
    body: Vec<u8>,
    fails: bool,
    posts: Mutex<Vec<Recorded>>,
}

#[derive(Clone, Debug)]
pub(super) struct Recorded {
    pub(super) url: String,
    pub(super) body: Vec<u8>,
}

impl Fixture {
    pub(super) fn make(
        kind: HandlerKind,
        headers: &BTreeMap<String, String>,
        body: Vec<u8>,
        fails: bool,
    ) -> Result<Arc<Self>, Box<HelperError>> {
        Ok(Arc::new(Self {
            kind,
            headers: header_map(headers)?,
            body,
            fails,
            posts: Mutex::new(Vec::new()),
        }))
    }

    pub(super) fn first_post(&self) -> Value {
        self.posts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .first()
            .map_or(
                Value::Null,
                |record| json!({"url": record.url, "body": record.body}),
            )
    }
}

#[async_trait]
impl StreamHandler for Fixture {
    fn kind(&self) -> HandlerKind {
        self.kind
    }

    async fn post(&self, request: PostRequest) -> Result<StreamHead, TransportError> {
        self.posts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                url: request.url.as_str().to_owned(),
                body: request.body,
            });
        let mut reads = vec![Ok(self.body.clone())];
        if self.fails {
            reads.push(Err(HttpError::Timeout));
        }
        Ok(StreamHead {
            status: 200,
            headers: self.headers.clone(),
            chunks: stream::iter(reads).boxed(),
        })
    }
}
