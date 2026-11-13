use async_trait::async_trait;
use betterlite_models::call_helpers::headers::{
    AdditionalHeaders, HeaderSource, process_response_headers,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessagesStreamHiddenParams {
    pub additional_headers: AdditionalHeaders,
}

impl From<&IndexMap<String, String>> for MessagesStreamHiddenParams {
    fn from(headers: &IndexMap<String, String>) -> Self {
        Self {
            additional_headers: process_response_headers(HeaderSource::Upstream(headers)),
        }
    }
}

#[async_trait]
pub trait MessagesProviderStream {
    type Error;

    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, Self::Error>;

    // None means the wrapped iterator has no aclose, so the view's close is a no-op
    async fn aclose(&mut self) -> Option<Result<(), Self::Error>> {
        None
    }

    // the iterator's attribute; the view reports true only for the literal boolean true
    fn buffered_provider_output(&self) -> Option<Json> {
        None
    }
}

pub struct MessagesStreamingResponse<S> {
    pub completion_stream: S,
    pub hidden_params: MessagesStreamHiddenParams,
}

impl<S: MessagesProviderStream> MessagesStreamingResponse<S> {
    pub fn has_buffered_provider_output(&self) -> bool {
        self.completion_stream
            .buffered_provider_output()
            .is_some_and(|reported| reported == Json::Bool(true))
    }
}

#[async_trait]
impl<S: MessagesProviderStream + Send> MessagesProviderStream for MessagesStreamingResponse<S> {
    type Error = S::Error;

    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, Self::Error> {
        self.completion_stream.next_chunk().await
    }

    async fn aclose(&mut self) -> Option<Result<(), Self::Error>> {
        self.completion_stream.aclose().await
    }

    fn buffered_provider_output(&self) -> Option<Json> {
        self.has_buffered_provider_output()
            .then_some(Json::Bool(true))
    }
}
