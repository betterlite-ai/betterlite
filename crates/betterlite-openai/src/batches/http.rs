use std::time::Duration;

use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_http::url_path::encode_segment;
use http::header::{ACCEPT, AUTHORIZATION};
use http::{HeaderMap, HeaderName, HeaderValue, Method};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;
use url::Url;

use crate::batches::error::SdkError;
use crate::batches::records::{BatchCreate, BatchPage, BatchQuery};
use crate::batches::sdk::{BatchesSdk, ClientConstructor, ClientKind, SdkArguments, Timeout};

use self::timeouts::phase_names;

mod timeouts;

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

// The OpenAI SDK over betterlite's pooled outbound client; max_retries is not applied because the pool never retries
#[derive(Debug, Clone)]
pub struct HttpBatchesClient {
    pub transport: ProxiedClient,
    pub kind: ClientKind,
    pub base_url: String,
    pub api_key: SecretString,
    pub organization: Option<String>,
    pub project: Option<String>,
    pub timeout: Duration,
}

// Builds a fresh client per call from the call's own settings, so no credential outlives or crosses a call
#[derive(Debug, Clone)]
pub struct HttpConstructor {
    pub transport: ProxiedClient,
    pub environment_key: Option<SecretString>,
    pub environment_organization: Option<String>,
    pub environment_project: Option<String>,
}

impl ClientConstructor for HttpConstructor {
    type Client = HttpBatchesClient;

    fn construct(
        &self,
        arguments: &SdkArguments,
        kind: ClientKind,
    ) -> Result<Option<HttpBatchesClient>, SdkError> {
        // an empty key is no key: the SDK raises Missing credentials before it sends anything
        let configured_key = arguments
            .api_key
            .clone()
            .or_else(|| self.environment_key.clone())
            .filter(|key| !key.expose_secret().is_empty());
        let Some(api_key) = configured_key else {
            return Ok(None);
        };
        let timeout = match &arguments.timeout {
            Some(Timeout::Seconds(seconds)) => Duration::try_from_secs_f64(*seconds)
                .map_err(|_source| SdkError::InvalidTimeout { seconds: *seconds })?,
            Some(Timeout::Phases(phases)) => {
                return Err(SdkError::UnsupportedTimeoutPhase {
                    phases: phase_names(phases),
                });
            }
            None => DEFAULT_TIMEOUT,
        };
        Ok(Some(HttpBatchesClient {
            transport: self.transport.clone(),
            kind,
            base_url: arguments
                .base_url
                .clone()
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_owned()),
            api_key,
            organization: arguments
                .organization
                .clone()
                .or_else(|| self.environment_organization.clone()),
            project: self.environment_project.clone(),
            timeout,
        }))
    }
}

struct Call<'call> {
    method: Method,
    path: String,
    query: Vec<(String, String)>,
    body: RequestBody,
    extra_headers: Option<&'call IndexMap<String, String>>,
    timeout: Option<f64>,
}

#[derive(Default)]
struct ErrorFields {
    error_type: Option<String>,
    param: Option<String>,
    code: Option<String>,
}

fn header_value(name: &str, value: &str) -> Result<HeaderValue, SdkError> {
    HeaderValue::try_from(value).map_err(|source| SdkError::Transport {
        source: TransportError::InvalidHeader {
            name: name.to_owned(),
            source,
        },
    })
}

// The SDK reads type, param and code from body["error"] when it is a mapping and from the body itself otherwise
fn error_fields(body: &[u8]) -> ErrorFields {
    let Ok(Value::Object(mut fields)) = serde_json::from_slice::<Value>(body) else {
        return ErrorFields::default();
    };
    let data = fields.remove("error").unwrap_or(Value::Object(fields));
    let Value::Object(inner) = data else {
        return ErrorFields::default();
    };
    ErrorFields {
        error_type: inner.get("type").and_then(Value::as_str).map(str::to_owned),
        param: inner
            .get("param")
            .and_then(Value::as_str)
            .map(str::to_owned),
        code: inner.get("code").and_then(Value::as_str).map(str::to_owned),
    }
}

fn status_error(response: &UpstreamResponse) -> SdkError {
    let fields = error_fields(&response.body);
    let body = str::from_utf8(&response.body).unwrap_or("<body is not UTF-8>");
    SdkError::Status {
        status: response.status,
        error_type: fields.error_type,
        param: fields.param,
        code: fields.code,
        message: format!("Error code: {} - {body}", response.status),
    }
}

impl HttpBatchesClient {
    async fn on_batch(
        &self,
        request: &BatchQuery,
        method: Method,
        suffix: &str,
        body: RequestBody,
    ) -> Result<Vec<u8>, SdkError> {
        let id = encode_segment(&request.batch_id, "batch_id")
            .map_err(|source| SdkError::Path { source })?;
        self.send(Call {
            method,
            path: format!("/batches/{id}{suffix}"),
            query: Vec::new(),
            body,
            extra_headers: request.extra_headers.as_ref(),
            timeout: request.timeout,
        })
        .await
    }

    fn headers(&self, extra: Option<&IndexMap<String, String>>) -> Result<HeaderMap, SdkError> {
        let mut headers = HeaderMap::new();
        let bearer = format!("Bearer {}", self.api_key.expose_secret());
        headers.insert(AUTHORIZATION, header_value("authorization", &bearer)?);
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        if let Some(organization) = &self.organization {
            headers.insert(
                HeaderName::from_static("openai-organization"),
                header_value("openai-organization", organization)?,
            );
        }
        if let Some(project) = &self.project {
            headers.insert(
                HeaderName::from_static("openai-project"),
                header_value("openai-project", project)?,
            );
        }
        for (name, value) in extra.into_iter().flatten() {
            let header_name =
                HeaderName::try_from(name.as_str()).map_err(|source| SdkError::HeaderName {
                    name: name.clone(),
                    source,
                })?;
            headers.insert(header_name, header_value(name, value)?);
        }
        Ok(headers)
    }

    async fn send(&self, call: Call<'_>) -> Result<Vec<u8>, SdkError> {
        let url = Url::parse(&format!(
            "{}{}",
            self.base_url.trim_end_matches('/'),
            call.path
        ))
        .map_err(|source| SdkError::BaseUrl { source })?;
        let deadline = match call.timeout {
            Some(seconds) => Duration::try_from_secs_f64(seconds)
                .map_err(|_source| SdkError::InvalidTimeout { seconds })?,
            None => self.timeout,
        };
        let response = self
            .transport
            .send_buffered(OutboundRequest {
                method: call.method,
                url,
                query: call.query,
                headers: self.headers(call.extra_headers)?,
                body: call.body,
                deadline,
            })
            .await
            .map_err(|source| SdkError::Transport { source })?;
        if (200..300).contains(&response.status) {
            return Ok(response.body);
        }
        Err(status_error(&response))
    }
}

impl BatchesSdk for HttpBatchesClient {
    type Page = BatchPage;

    fn kind(&self) -> ClientKind {
        self.kind
    }

    async fn create(&self, request: &BatchCreate) -> Result<Vec<u8>, SdkError> {
        let mut body =
            serde_json::to_value(request).map_err(|source| SdkError::Encode { source })?;
        if let Value::Object(fields) = &mut body {
            fields.retain(|name, _| {
                !matches!(name.as_str(), "extra_headers" | "timeout" | "extra_body")
            });
            let merged = request.extra_body.iter().flatten();
            fields.extend(merged.map(|(name, value)| (name.clone(), Value::String(value.clone()))));
        }
        let json = serde_json::to_vec(&body).map_err(|source| SdkError::Encode { source })?;
        self.send(Call {
            method: Method::POST,
            path: String::from("/batches"),
            query: Vec::new(),
            body: RequestBody::Json(json),
            extra_headers: request.extra_headers.as_ref(),
            timeout: request.timeout,
        })
        .await
    }

    async fn retrieve(&self, request: &BatchQuery) -> Result<Vec<u8>, SdkError> {
        self.on_batch(request, Method::GET, "", RequestBody::Empty)
            .await
    }

    async fn cancel(&self, request: &BatchQuery) -> Result<Vec<u8>, SdkError> {
        // the SDK sends extra_body as the cancel body and no body at all without it
        let body = match &request.extra_body {
            Some(extra) => RequestBody::Json(
                serde_json::to_vec(extra).map_err(|source| SdkError::Encode { source })?,
            ),
            None => RequestBody::Empty,
        };
        self.on_batch(request, Method::POST, "/cancel", body).await
    }

    async fn list(&self, after: Option<&str>, limit: Option<i64>) -> Result<BatchPage, SdkError> {
        let mut query = Vec::new();
        if let Some(value) = after {
            query.push((String::from("after"), value.to_owned()));
        }
        if let Some(count) = limit {
            query.push((String::from("limit"), count.to_string()));
        }
        let body = self
            .send(Call {
                method: Method::GET,
                path: String::from("/batches"),
                query,
                body: RequestBody::Empty,
                extra_headers: None,
                timeout: None,
            })
            .await?;
        serde_json::from_slice(&body).map_err(|source| SdkError::Body { source })
    }
}
