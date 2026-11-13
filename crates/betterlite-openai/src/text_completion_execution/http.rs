use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::header::{AUTHORIZATION, CONTENT_TYPE};
use http::{HeaderMap, HeaderName, HeaderValue, Method};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Map, Value};
use url::Url;

use crate::text_completion_transport::sdk::{
    ClientConstructor, ClientKind, RawCompletion, SdkArguments, SdkError, TextCompletionSdk,
};

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

// The provider answer the route needs besides the body it already got from the transport
#[derive(Debug, Clone, Default)]
pub struct UpstreamFacts {
    pub headers: HeaderMap,
    pub elapsed: Duration,
}

// The OpenAI SDK over betterlite's pooled outbound client; max_retries is not applied because the pool never retries
pub struct HttpCompletionConstructor {
    pub client: ProxiedClient,
    pub authorization: Option<SecretString>,
    pub facts: Arc<Mutex<Option<UpstreamFacts>>>,
}

impl ClientConstructor for HttpCompletionConstructor {
    type Client = HttpCompletionSdk;

    fn construct(
        &self,
        arguments: &SdkArguments,
        kind: ClientKind,
    ) -> Result<Option<Self::Client>, SdkError> {
        // a deployment without a credential fails the way the SDK's own missing-key error does
        let Some(authorization) = self.authorization.clone() else {
            return Ok(None);
        };
        let timeout =
            Duration::try_from_secs_f64(arguments.timeout).map_err(|_source| SdkError::Other {
                message: format!(
                    "the completion timeout {} is not a duration",
                    arguments.timeout
                ),
            })?;
        Ok(Some(HttpCompletionSdk {
            client: self.client.clone(),
            kind,
            authorization,
            base_url: arguments
                .base_url
                .clone()
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_owned()),
            organization: arguments.organization.clone(),
            timeout,
            facts: Arc::clone(&self.facts),
        }))
    }
}

#[derive(Clone)]
pub struct HttpCompletionSdk {
    client: ProxiedClient,
    kind: ClientKind,
    authorization: SecretString,
    base_url: String,
    organization: Option<String>,
    timeout: Duration,
    facts: Arc<Mutex<Option<UpstreamFacts>>>,
}

struct Prepared {
    url: Url,
    headers: HeaderMap,
    body: Vec<u8>,
}

fn header_value(name: &str, value: &str) -> Result<HeaderValue, SdkError> {
    HeaderValue::try_from(value).map_err(|source| SdkError::Other {
        message: format!("the {name} header value is not valid: {source}"),
    })
}

impl HttpCompletionSdk {
    fn prepare(&self, data: &Map<String, Value>) -> Result<Prepared, SdkError> {
        let mut body = data.clone();
        // the SDK reads extra_headers as request options, so they never become part of the JSON body
        let extra = body.remove("extra_headers");
        let url = Url::parse(&format!(
            "{}/completions",
            self.base_url.trim_end_matches('/')
        ))
        .map_err(|source| SdkError::Other {
            message: format!("the completion base URL is not a URL: {source}"),
        })?;
        let headers = self.request_headers(extra)?;
        let body = serde_json::to_vec(&Value::Object(body)).map_err(|source| SdkError::Other {
            message: format!("the completion request could not be written as JSON: {source}"),
        })?;
        Ok(Prepared { url, headers, body })
    }

    fn request_headers(&self, extra: Option<Value>) -> Result<HeaderMap, SdkError> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            AUTHORIZATION,
            header_value("authorization", self.authorization.expose_secret())?,
        );
        if let Some(organization) = &self.organization {
            headers.insert(
                HeaderName::from_static("openai-organization"),
                header_value("openai-organization", organization)?,
            );
        }
        if let Some(Value::Object(fields)) = extra {
            for (name, value) in fields {
                let Some(text) = value.as_str() else {
                    return Err(SdkError::Other {
                        message: format!("the extra header {name} is not a string"),
                    });
                };
                let header_name =
                    HeaderName::try_from(name.as_str()).map_err(|source| SdkError::Other {
                        message: format!("the extra header name {name} is not valid: {source}"),
                    })?;
                headers.insert(header_name, header_value(&name, text)?);
            }
        }
        Ok(headers)
    }
}

impl TextCompletionSdk for HttpCompletionSdk {
    fn kind(&self) -> ClientKind {
        self.kind
    }

    fn create(
        &self,
        data: &Map<String, Value>,
    ) -> impl Future<Output = Result<RawCompletion, SdkError>> + Send {
        let prepared = self.prepare(data);
        let client = self.client.clone();
        let facts = Arc::clone(&self.facts);
        let deadline = self.timeout;
        async move {
            let Prepared { url, headers, body } = prepared?;
            let started = Instant::now();
            let response = client
                .send_buffered(OutboundRequest {
                    method: Method::POST,
                    url,
                    query: Vec::new(),
                    headers,
                    body: RequestBody::Json(body),
                    deadline,
                })
                .await
                .map_err(|source| SdkError::Other {
                    message: source.to_string(),
                })?;
            let elapsed = started.elapsed();
            *facts.lock().unwrap_or_else(PoisonError::into_inner) = Some(UpstreamFacts {
                headers: response.headers.clone(),
                elapsed,
            });
            Ok(RawCompletion::Buffered {
                status: response.status,
                headers: response.headers,
                body: response.body,
            })
        }
    }
}
