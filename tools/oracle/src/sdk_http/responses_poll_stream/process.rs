use std::collections::VecDeque;
use std::fmt::Display;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_core::api::responses_poll_stream::ports::{
    PollChunkSource, PollProcessCall, PollProcessed, PollProcessorError, PollRequestProcessor,
};
use betterlite_http::error::HttpError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_models::decisions::Json;
use betterlite_openai::stream::SseParser;
use futures_util::StreamExt as _;
use futures_util::stream::BoxStream;
use http::header::{HeaderName, HeaderValue};
use http::{HeaderMap, Method};
use url::Url;

use crate::sdk_http::error::SdkHttpError;

const PROVIDER_TIMEOUT: Duration = Duration::from_secs(120);

// every provider-side failure reaches the worker as the message its Display names
trait ProcessorFailure<T> {
    fn processor_failure(self) -> Result<T, PollProcessorError>;
}

impl<T, E: Display> ProcessorFailure<T> for Result<T, E> {
    fn processor_failure(self) -> Result<T, PollProcessorError> {
        self.map_err(|error| PollProcessorError {
            message: error.to_string(),
        })
    }
}

// the provider call the worker makes: one POST to the routed deployment, its body framed for its iterator
pub(super) struct ProviderProcessor {
    client: ProxiedClient,
    url: Url,
}

impl ProviderProcessor {
    pub(super) fn new(client: ProxiedClient, api_base: &str) -> Result<Self, SdkHttpError> {
        let address = format!("{}/responses", api_base.trim_end_matches('/'));
        let url = Url::parse(&address).map_err(|source| SdkHttpError::Url(Box::new(source)))?;
        Ok(Self { client, url })
    }
}

#[async_trait]
impl PollRequestProcessor for ProviderProcessor {
    async fn process(
        &self,
        call: PollProcessCall<'_>,
    ) -> Result<PollProcessed, PollProcessorError> {
        let body = serde_json::to_vec(&Json::Object(call.data.clone())).processor_failure()?;
        let headers = request_headers(call.request.scope.headers.as_slice())?;
        let response = self
            .client
            .send(OutboundRequest {
                method: Method::POST,
                url: self.url.clone(),
                query: Vec::new(),
                headers,
                body: RequestBody::Json(body),
                deadline: PROVIDER_TIMEOUT,
            })
            .await
            .processor_failure()?;
        let status = response.response.status().as_u16();
        if !(200..300).contains(&status) {
            let message = format!("the provider answered with status {status}");
            return Err(PollProcessorError { message });
        }
        Ok(PollProcessed::Streaming(Box::new(ProviderChunks {
            body: response.body_stream(),
            parser: SseParser::default(),
            pending: VecDeque::new(),
            finished: false,
        })))
    }
}

// the detached request keeps the admitted scope, so its auth header reaches the provider unchanged
fn request_headers(headers: &[(String, String)]) -> Result<HeaderMap, PollProcessorError> {
    let mut map = HeaderMap::with_capacity(headers.len());
    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes()).processor_failure()?;
        let header_value = HeaderValue::from_bytes(value.as_bytes()).processor_failure()?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}

// the body iterator hands the worker one "data: " chunk per provider event
struct ProviderChunks {
    body: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    parser: SseParser,
    pending: VecDeque<Vec<u8>>,
    finished: bool,
}

#[async_trait]
impl PollChunkSource for ProviderChunks {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, PollProcessorError> {
        loop {
            if let Some(payload) = self.pending.pop_front() {
                let mut frame = b"data: ".to_vec();
                frame.extend_from_slice(&payload);
                frame.extend_from_slice(b"\n\n");
                return Ok(Some(frame));
            }
            if self.finished {
                return Ok(None);
            }
            let Some(chunk) = self.body.next().await else {
                self.finished = true;
                self.pending.extend(self.parser.finish());
                continue;
            };
            let bytes = chunk.map_err(|error| PollProcessorError {
                message: format!("the provider stream failed: {error}"),
            })?;
            self.pending
                .extend(self.parser.push(&bytes).processor_failure()?);
        }
    }
}
