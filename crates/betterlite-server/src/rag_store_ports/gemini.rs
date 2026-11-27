use std::error::Error as StdError;
use std::fmt::{Debug, Formatter, Result as FmtResult};
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::loader::{ConfigError, EnvironmentLookup};
use betterlite_config::sections::Section as _;
use betterlite_config::sections::studio_identity;
use betterlite_core::rag::gemini_ingestion::GeminiDefaults;
use betterlite_core::rag::gemini_ingestion::port::{
    GeminiReply, GeminiTransport, TransportBody, TransportError, TransportMethod, TransportRequest,
};
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::Method;
use http::header::HeaderValue;
use url::Url;

use super::exchange;

mod defaults_tests;
mod tests;

// B3454's transport over the RAG client the other RAG HTTP ports share
pub struct GeminiHttp {
    pub client: Arc<ProxiedClient>,
}

impl Debug for GeminiHttp {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("GeminiHttp").finish_non_exhaustive()
    }
}

#[async_trait]
impl GeminiTransport for GeminiHttp {
    async fn send(&self, request: TransportRequest) -> Result<GeminiReply, TransportError> {
        let TransportRequest {
            method,
            url,
            headers,
            body,
            timeout,
        } = request;
        let target = Url::parse(&url)
            .map_err(|source| transport_error(GeminiRequestError::Url { url, source }))?;
        let payload = request_body(body)?;
        let verb = match method {
            TransportMethod::Post => Method::POST,
            TransportMethod::Put => Method::PUT,
        };
        let reply = exchange(
            &self.client,
            OutboundRequest {
                method: verb,
                url: target,
                query: Vec::new(),
                headers,
                body: payload,
                deadline: timeout,
            },
        )
        .await
        .map_err(transport_error)?;
        Ok(GeminiReply {
            status: reply.status,
            headers: reply.headers,
            body: reply.body,
        })
    }
}

// B1331's Studio identity rules are the one place a RAG port reads the environment
pub fn defaults<Lookup>(environment: &Lookup) -> Result<GeminiDefaults, ConfigError>
where
    Lookup: EnvironmentLookup,
{
    let settings = studio_identity::Raw::default().convert(environment)?;
    Ok(GeminiDefaults {
        api_key: settings.environment_key,
        api_base: settings.api_base,
    })
}

// httpx sends a content= body without a type; the shared request builder always carries one
fn request_body(body: TransportBody) -> Result<RequestBody, TransportError> {
    match body {
        TransportBody::Json(value) => serde_json::to_vec(&value)
            .map(RequestBody::Json)
            .map_err(|source| transport_error(GeminiRequestError::Body { source })),
        TransportBody::Bytes(bytes) => Ok(RequestBody::Bytes {
            content_type: HeaderValue::from_static("application/octet-stream"),
            data: bytes,
        }),
    }
}

#[derive(Debug, thiserror::Error)]
enum GeminiRequestError {
    #[error("the request URL {url:?} is not a URL")]
    Url {
        url: String,
        #[source]
        source: url::ParseError,
    },
    #[error("the request body could not be encoded")]
    Body {
        #[source]
        source: serde_json::Error,
    },
}

fn transport_error<Failure>(failure: Failure) -> TransportError
where
    Failure: StdError + Send + Sync + 'static,
{
    TransportError {
        source: Box::new(failure),
    }
}
