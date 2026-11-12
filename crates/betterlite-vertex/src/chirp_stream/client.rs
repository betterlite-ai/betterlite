use std::future::{Future, ready};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

use google_cloud_auth::credentials::{CacheableResource, CredentialsProvider, EntityTag};
use google_cloud_auth::errors::CredentialsError;
use google_cloud_gax::streaming::{RequestSender, ResponseStream};
use google_cloud_speech_v2::client::Speech;
use google_cloud_speech_v2::model::{StreamingRecognizeRequest, StreamingRecognizeResponse};
use http::header::{AUTHORIZATION, HeaderValue};
use http::{Extensions, HeaderMap};
use secrecy::{ExposeSecret as _, SecretString};

use super::error::ChirpStreamError;
use super::target::SpeechStreamingTarget;

mod tests;

pub type ClientFuture<'client, T> = Pin<Box<dyn Future<Output = T> + Send + 'client>>;

pub trait RequestSink: Send + Sync {
    fn send(
        &mut self,
        request: StreamingRecognizeRequest,
    ) -> ClientFuture<'_, Result<(), ChirpStreamError>>;
}

pub trait ResponseSource: Send {
    fn next_response(
        &mut self,
    ) -> ClientFuture<'_, Option<Result<StreamingRecognizeResponse, ChirpStreamError>>>;
}

pub struct RecognizedStream {
    pub requests: Box<dyn RequestSink>,
    pub responses: Box<dyn ResponseSource>,
}

pub trait SpeechStreamingClient: Send + Sync {
    fn streaming_recognize(&self) -> Result<RecognizedStream, ChirpStreamError>;
    fn close(&self);
}

#[derive(Debug)]
struct StaticToken {
    token: SecretString,
}

impl CredentialsProvider for StaticToken {
    fn headers(
        &self,
        _extensions: Extensions,
    ) -> impl Future<Output = Result<CacheableResource<HeaderMap>, CredentialsError>> + Send {
        ready(bearer_headers(&self.token))
    }

    fn universe_domain(&self) -> impl Future<Output = Option<String>> + Send {
        ready(None)
    }
}

fn bearer_headers(token: &SecretString) -> Result<CacheableResource<HeaderMap>, CredentialsError> {
    let mut header = HeaderValue::from_str(&format!("Bearer {}", token.expose_secret()))
        .map_err(|source| CredentialsError::from_source(false, source))?;
    header.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, header);
    Ok(CacheableResource::New {
        entity_tag: EntityTag::new(),
        data: headers,
    })
}

struct SdkRequestSink {
    sender: RequestSender<StreamingRecognizeRequest>,
}

impl RequestSink for SdkRequestSink {
    fn send(
        &mut self,
        request: StreamingRecognizeRequest,
    ) -> ClientFuture<'_, Result<(), ChirpStreamError>> {
        Box::pin(async move {
            self.sender
                .send(request)
                .await
                .map_err(ChirpStreamError::Send)
        })
    }
}

struct SdkResponseSource {
    responses: ResponseStream<StreamingRecognizeResponse>,
}

impl ResponseSource for SdkResponseSource {
    fn next_response(
        &mut self,
    ) -> ClientFuture<'_, Option<Result<StreamingRecognizeResponse, ChirpStreamError>>> {
        Box::pin(async move {
            self.responses
                .next()
                .await
                .map(|item| item.map_err(ChirpStreamError::Response))
        })
    }
}

pub struct SdkSpeechClient {
    speech: Mutex<Option<Speech>>,
    closed: AtomicBool,
}

impl SpeechStreamingClient for SdkSpeechClient {
    fn streaming_recognize(&self) -> Result<RecognizedStream, ChirpStreamError> {
        let guard = self.speech.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(speech) = guard.as_ref() else {
            return Err(ChirpStreamError::ClientClosed);
        };
        let stream = speech.streaming_recognize().build();
        drop(guard);
        let (sender, responses) = stream;
        Ok(RecognizedStream {
            requests: Box::new(SdkRequestSink { sender }),
            responses: Box::new(SdkResponseSource { responses }),
        })
    }

    fn close(&self) {
        if !self.closed.swap(true, Ordering::SeqCst) {
            let speech = self
                .speech
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            drop(speech);
        }
    }
}

// LiteLLM's target carries a bare host and grpc_asyncio dials it over TLS on 443
fn speech_endpoint(api_endpoint: &str) -> String {
    if api_endpoint.contains("://") {
        api_endpoint.to_owned()
    } else {
        format!("https://{api_endpoint}")
    }
}

pub async fn open_speech_client(
    target: &SpeechStreamingTarget,
    access_token: SecretString,
) -> Result<SdkSpeechClient, ChirpStreamError> {
    let speech = Speech::builder()
        .with_endpoint(speech_endpoint(&target.api_endpoint))
        .with_credentials(StaticToken {
            token: access_token,
        })
        .build()
        .await
        .map_err(ChirpStreamError::Open)?;
    Ok(SdkSpeechClient {
        speech: Mutex::new(Some(speech)),
        closed: AtomicBool::new(false),
    })
}
