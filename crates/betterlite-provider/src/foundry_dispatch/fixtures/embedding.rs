use std::future::{Future, ready};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_azure::image_transport::{ImageReply, ImageRequest, ImageTransport};
use betterlite_foundry::embeddings::call::{
    EmbeddingClientFactory, TextEmbedding, TextEmbeddingCall,
};
use betterlite_foundry::embeddings::error::FoundryEmbeddingError;
use betterlite_foundry::embeddings::reply::EmbeddingReply;
use betterlite_foundry::embeddings::request::FoundryEmbeddingOptions;
use betterlite_http::client_kind::ClientKind;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_openai::embedding::EmbeddingRecord;
use secrecy::ExposeSecret as _;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageSent {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

#[derive(Clone)]
pub struct ImageClient {
    pub status: u16,
    pub body: String,
    pub sent: Arc<Mutex<Vec<ImageSent>>>,
}

fn record<S>(sent: &Mutex<Vec<S>>, entry: S) {
    let mut recorded = sent.lock().unwrap_or_else(PoisonError::into_inner);
    recorded.push(entry);
}

impl ImageTransport for ImageClient {
    fn send(
        &self,
        request: ImageRequest,
    ) -> impl Future<Output = Result<ImageReply, TransportError>> + Send {
        record(
            &self.sent,
            ImageSent {
                method: request.method.as_str().to_owned(),
                url: request.url,
                headers: request
                    .headers
                    .iter()
                    .map(|(name, value)| {
                        (
                            name.as_str().to_owned(),
                            value.to_str().unwrap_or_default().to_owned(),
                        )
                    })
                    .collect(),
                body: String::from_utf8(request.body).unwrap_or_default(),
            },
        );
        ready(Ok(ImageReply {
            status: self.status,
            headers: http::HeaderMap::new(),
            body: self.body.clone().into_bytes(),
        }))
    }
}

pub struct Factory {
    pub status: u16,
    pub image_body: String,
    pub sent: Arc<Mutex<Vec<ImageSent>>>,
}

impl EmbeddingClientFactory for Factory {
    type Client = ImageClient;

    fn sync_client(&self, _timeout: Duration) -> Self::Client {
        ImageClient {
            status: self.status,
            body: self.image_body.clone(),
            sent: Arc::clone(&self.sent),
        }
    }

    fn async_client(&self, _timeout: Duration) -> Self::Client {
        self.sync_client(Duration::ZERO)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextSent {
    pub mode: ClientKind,
    pub model: String,
    pub input: Vec<String>,
    pub api_base: Option<String>,
    pub api_key: Option<String>,
    pub options: FoundryEmbeddingOptions,
}

// The inherited OpenAI embedding call belongs to another task; this stands in at its seam
pub struct Text {
    pub data: Vec<EmbeddingRecord>,
    pub model: Option<String>,
    pub sent: Mutex<Vec<TextSent>>,
}

impl Text {
    pub fn snapshot(&self) -> MutexGuard<'_, Vec<TextSent>> {
        self.sent.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl TextEmbedding for Text {
    fn embed(
        &self,
        call: TextEmbeddingCall<'_>,
    ) -> impl Future<Output = Result<EmbeddingReply, FoundryEmbeddingError>> + Send {
        let reply = EmbeddingReply {
            object: Some(String::from("list")),
            model: Some(self.model.clone()),
            data: Some(Some(self.data.clone())),
            usage: None,
            additional_headers: AdditionalHeaders::new(),
        };
        let sent = TextSent {
            mode: call.mode,
            model: call.model.to_owned(),
            input: call.input.to_vec(),
            api_base: call.api_base.map(str::to_owned),
            api_key: call.api_key.map(|key| key.expose_secret().to_owned()),
            options: call.options.clone(),
        };
        record(&self.sent, sent);
        ready(Ok(reply))
    }
}
