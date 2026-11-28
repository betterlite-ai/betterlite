use std::future::Future;
use std::time::Duration;

use betterlite_azure::image_transport::{ImageRequest, ImageTransport};
use betterlite_http::client_kind::{ClientKind, SuppliedClient};
use betterlite_models::call_helpers::headers::{AdditionalHeaders, OPENAI_RESPONSE_HEADERS};
use betterlite_models::decisions::Json;
use betterlite_openai::embedding::EmbeddingRecord;
use betterlite_openai::files::stream::flatten_headers;
use http::Method;
use http::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};

use crate::embeddings::cohere::transform_response;
use crate::embeddings::error::FoundryEmbeddingError;
use crate::embeddings::reply::{
    EmbeddingReply, FoundryEmbeddingResponse, decode_foundry_embedding, interleave,
};
use crate::embeddings::request::{
    FoundryEmbeddingOptions, ImageEmbeddingRequest, transform_request,
};
use crate::embeddings::route::models_route_base;

// B173 builds the clients: HTTPHandler(timeout, concurrent_limit=1) sync, azure_ai async
pub trait EmbeddingClientFactory: Sync {
    type Client: ImageTransport;

    fn sync_client(&self, timeout: Duration) -> Self::Client;

    fn async_client(&self, timeout: Duration) -> Self::Client;
}

pub struct TextEmbeddingCall<'call> {
    pub mode: ClientKind,
    pub model: &'call str,
    pub input: &'call [String],
    pub api_base: Option<&'call str>,
    pub api_key: Option<&'call SecretString>,
    pub options: &'call FoundryEmbeddingOptions,
    pub timeout: Duration,
}

// the inherited OpenAI embedding call sees the original input and returns projected headers
pub trait TextEmbedding: Sync {
    fn embed(
        &self,
        call: TextEmbeddingCall<'_>,
    ) -> impl Future<Output = Result<EmbeddingReply, FoundryEmbeddingError>> + Send;
}

pub struct FoundryEmbeddingRequest<'call> {
    pub model: &'call str,
    pub input: &'call [String],
    pub options: FoundryEmbeddingOptions,
    pub api_base: Option<&'call str>,
    pub api_key: Option<&'call SecretString>,
    pub timeout: Duration,
    pub mode: ClientKind,
}

pub async fn embedding<Factory, Text>(
    request: &FoundryEmbeddingRequest<'_>,
    supplied: Option<SuppliedClient<'_, Factory::Client>>,
    factory: &Factory,
    text: &Text,
) -> Result<FoundryEmbeddingResponse, FoundryEmbeddingError>
where
    Factory: EmbeddingClientFactory,
    Text: TextEmbedding,
{
    let route_base = models_route_base(request.api_base);
    let base = route_base.as_deref();
    let plan = transform_request(request.input, &request.options);
    let mut response = FoundryEmbeddingResponse::default();
    let image_data = if plan.image.input.is_empty() {
        None
    } else {
        let mut reply = image_embedding(request, base, &plan.image, supplied, factory).await?;
        let data = reply_data(&mut reply, FoundryEmbeddingError::ImageDataMissing)?;
        response.apply_reply(reply);
        Some(data)
    };
    let text_data = text_embedding(request, base, &plan.text_input, text, &mut response).await?;
    response.data = interleave(
        request.input.len(),
        &plan.image_indices,
        image_data.as_deref(),
        text_data.as_deref(),
    )?;
    transform_response(&mut response)?;
    Ok(response)
}

async fn image_embedding<Factory>(
    request: &FoundryEmbeddingRequest<'_>,
    base: Option<&str>,
    image: &ImageEmbeddingRequest<'_>,
    supplied: Option<SuppliedClient<'_, Factory::Client>>,
    factory: &Factory,
) -> Result<EmbeddingReply, FoundryEmbeddingError>
where
    Factory: EmbeddingClientFactory,
{
    sync_guards(request.mode, base, request.api_key)?;
    let url = format!("{}/images/embeddings", base.unwrap_or_default());
    let headers = request_headers(request.api_key)?;
    let body =
        serde_json::to_vec(image).map_err(|source| FoundryEmbeddingError::Encode { source })?;
    let image_request = ImageRequest::new(Method::POST, url.clone(), headers, body);
    let outcome = if let Some(reused) = supplied.filter(|client| client.kind == request.mode) {
        reused.transport.send(image_request).await
    } else {
        let built = match request.mode {
            ClientKind::Sync => factory.sync_client(request.timeout),
            ClientKind::Asynchronous => factory.async_client(request.timeout),
        };
        built.send(image_request).await
    };
    let sent = outcome.map_err(|source| FoundryEmbeddingError::Transport { url, source })?;
    // LiteLLM's HTTP client raises for any non-2xx status before the body is read as an embedding
    if !(200..300).contains(&sent.status) {
        return Err(FoundryEmbeddingError::Provider {
            status: sent.status,
        });
    }
    let mut reply = decode_foundry_embedding(&sent.body)?;
    reply.additional_headers = projected_headers(&sent.headers);
    Ok(reply)
}

async fn text_embedding<Text>(
    request: &FoundryEmbeddingRequest<'_>,
    base: Option<&str>,
    text_input: &[&str],
    text: &Text,
    response: &mut FoundryEmbeddingResponse,
) -> Result<Option<Vec<EmbeddingRecord>>, FoundryEmbeddingError>
where
    Text: TextEmbedding,
{
    if text_input.is_empty() {
        return Ok(None);
    }
    let mut reply = text
        .embed(TextEmbeddingCall {
            mode: request.mode,
            model: request.model,
            input: request.input,
            api_base: base,
            api_key: request.api_key,
            options: &request.options,
            timeout: request.timeout,
        })
        .await?;
    let data = reply_data(&mut reply, FoundryEmbeddingError::TextDataMissing)?;
    response.apply_reply(reply);
    Ok(Some(data))
}

// the sync route refuses a missing base or key; the async route leaves both to the request itself
fn sync_guards(
    mode: ClientKind,
    base: Option<&str>,
    key: Option<&SecretString>,
) -> Result<(), FoundryEmbeddingError> {
    if mode != ClientKind::Sync {
        return Ok(());
    }
    if base.is_none() {
        return Err(FoundryEmbeddingError::ApiBaseMissing);
    }
    if key.is_none() {
        return Err(FoundryEmbeddingError::ApiKeyMissing);
    }
    Ok(())
}

fn request_headers(api_key: Option<&SecretString>) -> Result<HeaderMap, FoundryEmbeddingError> {
    let key = api_key.map_or("", |secret| secret.expose_secret());
    let mut headers = HeaderMap::new();
    let value = HeaderValue::from_str(&format!("Bearer {key}")).map_err(|source| {
        FoundryEmbeddingError::Header {
            name: "authorization",
            source: http::Error::from(source),
        }
    })?;
    headers.insert(AUTHORIZATION, value);
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    Ok(headers)
}

fn reply_data(
    reply: &mut EmbeddingReply,
    missing: FoundryEmbeddingError,
) -> Result<Vec<EmbeddingRecord>, FoundryEmbeddingError> {
    reply
        .data
        .take()
        .ok_or(FoundryEmbeddingError::DataMissing)?
        .ok_or(missing)
}

// LiteLLM's get_response_headers repeats the limit and remaining names bare, never the reset pair
#[must_use]
pub fn projected_headers(headers: &HeaderMap) -> AdditionalHeaders {
    let mut pairs = AdditionalHeaders::new();
    for (name, value) in flatten_headers(headers) {
        let text = Json::Text(value);
        if OPENAI_RESPONSE_HEADERS.contains(&name.as_str()) && !name.contains("reset") {
            pairs.insert(name.clone(), text.clone());
        }
        let key = if name.contains("llm_provider") {
            name
        } else {
            format!("llm_provider-{name}")
        };
        pairs.insert(key, text);
    }
    pairs
}
