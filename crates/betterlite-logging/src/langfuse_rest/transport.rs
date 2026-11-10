use std::error::Error as StdError;
use std::str::{Utf8Error, from_utf8};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use http::Method;
use http::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;
use url::{ParseError, Url};

// LiteLLM pins the langfuse release in its lock file; its generated client sends that identity on every call
pub const LANGFUSE_SDK_NAME: &str = "python";
pub const LANGFUSE_SDK_VERSION: &str = "4.15.2";

const SDK_NAME_HEADER: &str = "x-langfuse-sdk-name";
const SDK_VERSION_HEADER: &str = "x-langfuse-sdk-version";
const PUBLIC_KEY_HEADER: &str = "x-langfuse-public-key";
const FERN_LANGUAGE_HEADER: &str = "x-fern-language";

#[derive(Debug, Error)]
pub enum RequestFailure {
    #[error("the Langfuse URL {url} is not valid")]
    Url {
        url: String,
        #[source]
        source: ParseError,
    },
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error(transparent)]
    Decode(TransportError),
}

#[derive(Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn json(&self) -> Result<Json, Utf8Error> {
        serde_json::from_slice(&self.body).map_or_else(
            |_error| from_utf8(&self.body).map(|text| Json::Text(text.to_owned())),
            Ok,
        )
    }
}

pub struct LangfuseHttp {
    pub client: ProxiedClient,
    pub base_url: String,
    pub public_key: Option<String>,
    pub secret_key: Option<SecretString>,
    pub timeout: Duration,
}

impl LangfuseHttp {
    pub async fn get(
        &self,
        path: &str,
        query: Vec<(String, String)>,
    ) -> Result<HttpResponse, RequestFailure> {
        let endpoint = if self.base_url.ends_with('/') {
            self.base_url.clone()
        } else {
            format!("{}/", self.base_url)
        };
        let base = Url::parse(&endpoint).map_err(|source| RequestFailure::Url {
            url: endpoint,
            source,
        })?;
        let url = base.join(path).map_err(|source| RequestFailure::Url {
            url: path.to_owned(),
            source,
        })?;
        let request = OutboundRequest {
            method: Method::GET,
            url,
            query,
            headers: headers_for(self.public_key.as_deref(), self.secret_key.as_ref())?,
            body: RequestBody::Empty,
            deadline: self.timeout,
        };
        let stream = self.client.send_following(request).await?;
        let response = stream
            .buffered(self.client.limits.body_bytes)
            .await
            .map_err(buffered_failure)?;
        Ok(HttpResponse {
            status: response.status,
            body: response.body,
        })
    }
}

fn buffered_failure(error: TransportError) -> RequestFailure {
    if let TransportError::Body(source) = &error
        && !contains_hyper_error(source)
    {
        return RequestFailure::Decode(error);
    }
    RequestFailure::Transport(error)
}

// tower-http returns a dropped connection as the original hyper body error; only a decompressor failure has none
fn contains_hyper_error(error: &(dyn StdError + 'static)) -> bool {
    let mut current = Some(error);
    while let Some(source) = current {
        if source.downcast_ref::<hyper::Error>().is_some() {
            return true;
        }
        current = source.source();
    }
    false
}

pub fn headers_for(
    public_key: Option<&str>,
    secret_key: Option<&SecretString>,
) -> Result<HeaderMap, TransportError> {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static(FERN_LANGUAGE_HEADER),
        HeaderValue::from_static("Python"),
    );
    headers.insert(
        HeaderName::from_static(SDK_NAME_HEADER),
        HeaderValue::from_static(LANGFUSE_SDK_NAME),
    );
    headers.insert(
        HeaderName::from_static(SDK_VERSION_HEADER),
        HeaderValue::from_static(LANGFUSE_SDK_VERSION),
    );
    if let Some(key) = public_key {
        let value = HeaderValue::try_from(key).map_err(|source| TransportError::InvalidHeader {
            name: PUBLIC_KEY_HEADER.to_owned(),
            source,
        })?;
        headers.insert(HeaderName::from_static(PUBLIC_KEY_HEADER), value);
    }
    if let (Some(key), Some(secret)) = (public_key, secret_key) {
        let credentials = format!("{key}:{}", secret.expose_secret());
        let mut value = HeaderValue::try_from(format!("Basic {}", STANDARD.encode(credentials)))
            .map_err(|source| TransportError::InvalidHeader {
                name: "authorization".to_owned(),
                source,
            })?;
        value.set_sensitive(true);
        headers.insert(AUTHORIZATION, value);
    }
    Ok(headers)
}
