use std::fmt::{Debug, Formatter, Result as FmtResult};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_core::rag::vertex_ingestion::ports as vertex_ports;
use betterlite_core::rag::vertex_ingestion::ports::{
    GetRequest, HttpReply, JsonRequest, MultipartRequest, PortError,
};
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use http::{HeaderName, Method};
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};
use url::Url;

use super::exchange;

mod tests;

const UPLOAD_PROTOCOL: HeaderName = HeaderName::from_static("x-goog-upload-protocol");

// B3455's three calls over the RAG client the other RAG HTTP ports share
pub struct VertexRagHttp {
    pub client: Arc<ProxiedClient>,
}

impl Debug for VertexRagHttp {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("VertexRagHttp").finish_non_exhaustive()
    }
}

#[async_trait]
impl vertex_ports::VertexRagHttp for VertexRagHttp {
    async fn post_json(&self, request: JsonRequest<'_>) -> Result<HttpReply, PortError> {
        self.send(wire(Call::Json(request))?).await
    }

    async fn get(&self, request: GetRequest<'_>) -> Result<HttpReply, PortError> {
        self.send(wire(Call::Get(request))?).await
    }

    async fn post_multipart(&self, request: MultipartRequest<'_>) -> Result<HttpReply, PortError> {
        self.send(wire(Call::Multipart(request))?).await
    }
}

struct RagWire<'request> {
    method: Method,
    url: &'request str,
    headers: HeaderMap,
    body: RequestBody,
    timeout: Duration,
}

enum Call<'request> {
    Json(JsonRequest<'request>),
    Get(GetRequest<'request>),
    Multipart(MultipartRequest<'request>),
}

fn wire(call: Call<'_>) -> Result<RagWire<'_>, PortError> {
    match call {
        Call::Json(request) => Ok(RagWire {
            method: Method::POST,
            url: request.url,
            headers: authorized(request.token)?,
            body: RequestBody::Json(request.body.to_vec()),
            timeout: request.timeout,
        }),
        Call::Get(request) => Ok(RagWire {
            method: Method::GET,
            url: request.url,
            headers: authorized(request.token)?,
            body: RequestBody::Empty,
            timeout: request.timeout,
        }),
        Call::Multipart(request) => {
            let mut headers = authorized(request.token)?;
            headers.insert(UPLOAD_PROTOCOL, HeaderValue::from_static("multipart"));
            Ok(RagWire {
                method: Method::POST,
                url: request.url,
                headers,
                body: multipart_body(&request),
                timeout: request.timeout,
            })
        }
    }
}

impl VertexRagHttp {
    async fn send(&self, wire: RagWire<'_>) -> Result<HttpReply, PortError> {
        let target = Url::parse(wire.url).map_err(|source| {
            PortError::new(VertexRequestError::Url {
                url: wire.url.to_owned(),
                source,
            })
        })?;
        let reply = exchange(
            &self.client,
            OutboundRequest {
                method: wire.method,
                url: target,
                query: Vec::new(),
                headers: wire.headers,
                body: wire.body,
                deadline: wire.timeout,
            },
        )
        .await
        .map_err(|source| PortError::new(VertexRequestError::Transport { source }))?;
        Ok(HttpReply {
            status: reply.status,
            body: reply.body,
        })
    }
}

#[derive(Debug, thiserror::Error)]
enum VertexRequestError {
    #[error("the request URL {url:?} is not a URL")]
    Url {
        url: String,
        #[source]
        source: url::ParseError,
    },
    #[error("the bearer token is not a header value")]
    Token {
        #[source]
        source: http::header::InvalidHeaderValue,
    },
    #[error("the request could not be sent")]
    Transport {
        #[source]
        source: TransportError,
    },
}

fn authorized(token: &SecretString) -> Result<HeaderMap, PortError> {
    let bearer = format!("Bearer {}", token.expose_secret());
    let mut value = HeaderValue::from_str(&bearer)
        .map_err(|source| PortError::new(VertexRequestError::Token { source }))?;
    value.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, value);
    Ok(headers)
}

fn multipart_body(request: &MultipartRequest<'_>) -> RequestBody {
    RequestBody::Multipart {
        boundary: multipart_boundary(request.metadata, request.file),
        parts: vec![
            MultipartPart {
                name: "metadata".to_owned(),
                file_name: None,
                content_type: Some("application/json".to_owned()),
                data: request.metadata.to_vec(),
            },
            MultipartPart {
                name: "file".to_owned(),
                file_name: Some(request.filename.to_owned()),
                content_type: Some(request.file_content_type.to_owned()),
                data: request.file.to_vec(),
            },
        ],
    }
}

// the boundary is the request's own digest, so it never repeats inside the data and needs no random source
fn multipart_boundary(metadata: &[u8], file: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(metadata);
    hasher.update(file);
    format!("betterlite-{}", hex::encode(hasher.finalize()))
}
