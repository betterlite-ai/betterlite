use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::fine_tuning_read::{
    AzureReadCall, FineTuningReadDispatch, OpenAiReadCall, ReadOperation, ReadTarget, ReadValues,
};
use betterlite_core::api::resource_sdk_settings::timeout::FineTuningTimeout;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::fine_tuning::client::SdkError;
use http::Method;
use http::header::{HeaderMap, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;
use url::Url;

use super::super::error::SdkHttpError;

// LiteLLM's Azure client asks for this version when neither the call nor the environment names one
const AZURE_DEFAULT_API_VERSION: &str = "2025-02-01-preview";
// LiteLLM treats exactly these names as the Azure v1 surface, matched case-sensitively
const V1_API_VERSIONS: [&str; 3] = ["preview", "latest", "v1"];
const SUPPLIED_KEY: &str = "sk-supplied";
const SUPPLIED_ORGANIZATION: &str = "org-supplied";
const READ_DEADLINE: Duration = Duration::from_secs(600);

#[derive(Debug, thiserror::Error)]
pub(super) enum ReadWireError {
    #[error(transparent)]
    Sdk {
        #[source]
        source: SdkError,
    },
    #[error("the provider request failed: {0}")]
    Transport(#[source] Box<TransportError>),
    #[error("the provider request URL is not valid: {0}")]
    Url(#[source] Box<url::ParseError>),
    #[error("the provider header {name} is not valid: {source}")]
    Header {
        name: &'static str,
        #[source]
        source: http::header::InvalidHeaderValue,
    },
    #[error("the azure read needs an api_base")]
    MissingBase,
    #[error("the provider error body does not decode: {0}")]
    Body(#[source] serde_json::Error),
}

#[derive(Clone)]
pub(super) struct HttpDispatch {
    pub(super) client: ProxiedClient,
    // LiteLLM hands a supplied client to its openai handler only, so azure ignores it
    pub(super) supplied: bool,
}

pub(super) fn outbound(env: &BTreeMap<String, String>) -> Result<ProxiedClient, SdkHttpError> {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), env).map_err(SdkHttpError::ProxyPolicy)?,
    );
    ProxiedClient::outbound(&policy).map_err(|error| SdkHttpError::Client(Box::new(error)))
}

impl FineTuningReadDispatch for HttpDispatch {
    type Error = ReadWireError;

    async fn read(&self, target: ReadTarget<'_>) -> Result<Vec<u8>, Self::Error> {
        let request = match target {
            ReadTarget::OpenAi(call) => self.openai_request(&call)?,
            ReadTarget::Azure(call) => azure_request(&call)?,
        };
        let response = self
            .client
            .send_buffered(request)
            .await
            .map_err(|source| ReadWireError::Transport(Box::new(source)))?;
        if response.status >= 400 {
            return Err(ReadWireError::Sdk {
                source: sdk_error(&response.body, response.status)?,
            });
        }
        Ok(response.body)
    }
}

impl HttpDispatch {
    // a supplied client carries its own base, key and organization; the call's base only locates it
    fn openai_request(&self, call: &OpenAiReadCall<'_>) -> Result<OutboundRequest, ReadWireError> {
        let mut headers = HeaderMap::new();
        let base = if self.supplied {
            let key = SecretString::from(SUPPLIED_KEY);
            insert_header(
                &mut headers,
                "authorization",
                &format!("Bearer {}", key.expose_secret()),
            )?;
            insert_header(&mut headers, "openai-organization", SUPPLIED_ORGANIZATION)?;
            format!("{}/supplied", call.api_base)
        } else {
            if let Some(key) = call.api_key {
                insert_header(
                    &mut headers,
                    "authorization",
                    &format!("Bearer {}", key.expose_secret()),
                )?;
            }
            if let Some(organization) = call.organization {
                insert_header(&mut headers, "openai-organization", organization)?;
            }
            call.api_base.clone()
        };
        let (url, query) = openai_target(&base, &call.values)?;
        Ok(OutboundRequest {
            method: Method::GET,
            url,
            query,
            headers,
            body: RequestBody::Empty,
            deadline: deadline(&call.values),
        })
    }
}

fn openai_target(
    api_base: &str,
    values: &ReadValues<'_>,
) -> Result<(Url, Vec<(String, String)>), ReadWireError> {
    let suffix = suffix(values.operation);
    let url = Url::parse(&format!(
        "{}/fine_tuning/jobs{suffix}",
        api_base.trim_end_matches('/')
    ))
    .map_err(|source| ReadWireError::Url(Box::new(source)))?;
    Ok((url, paging(values.operation)))
}

fn azure_request(call: &AzureReadCall<'_>) -> Result<OutboundRequest, ReadWireError> {
    let Some(api_base) = call.api_base else {
        return Err(ReadWireError::MissingBase);
    };
    let version = call
        .api_version
        .filter(|value| !value.is_empty())
        .unwrap_or(AZURE_DEFAULT_API_VERSION);
    let suffix = suffix(call.values.operation);
    let mut headers = HeaderMap::new();
    if let Some(key) = call.api_key {
        insert_header(&mut headers, "api-key", key.expose_secret())?;
    }
    let (url, mut query) = if V1_API_VERSIONS.contains(&version) {
        // the v1 surface is a plain OpenAI client over /openai/v1/, with no api-version query
        let url = Url::parse(&format!("{api_base}/openai/v1/fine_tuning/jobs{suffix}"))
            .map_err(|source| ReadWireError::Url(Box::new(source)))?;
        (url, Vec::new())
    } else {
        let url = Url::parse(&format!(
            "{}/openai/fine_tuning/jobs{suffix}",
            api_base.trim_end_matches('/')
        ))
        .map_err(|source| ReadWireError::Url(Box::new(source)))?;
        (url, vec![("api-version".to_owned(), version.to_owned())])
    };
    query.extend(paging(call.values.operation));
    Ok(OutboundRequest {
        method: Method::GET,
        url,
        query,
        headers,
        body: RequestBody::Empty,
        deadline: deadline(&call.values),
    })
}

fn suffix(operation: ReadOperation<'_>) -> String {
    match operation {
        ReadOperation::List { .. } => String::new(),
        ReadOperation::Retrieve { job_id } => format!("/{job_id}"),
    }
}

// the SDK drops a null, empty or missing pagination value and sends a zero limit as "0"
fn paging(operation: ReadOperation<'_>) -> Vec<(String, String)> {
    let ReadOperation::List { after, limit } = operation else {
        return Vec::new();
    };
    let mut query = Vec::new();
    if let Some(after) = after.filter(|value| !value.is_empty()) {
        query.push(("after".to_owned(), after.to_owned()));
    }
    if let Some(limit) = limit {
        query.push(("limit".to_owned(), limit.to_string()));
    }
    query
}

fn insert_header(
    headers: &mut HeaderMap,
    name: &'static str,
    value: &str,
) -> Result<(), ReadWireError> {
    let parsed =
        HeaderValue::from_str(value).map_err(|source| ReadWireError::Header { name, source })?;
    headers.insert(http::header::HeaderName::from_static(name), parsed);
    Ok(())
}

fn deadline(values: &ReadValues<'_>) -> Duration {
    match values.timeout {
        FineTuningTimeout::Seconds(seconds) => Duration::try_from_secs_f64(*seconds)
            .ok()
            .filter(|duration| !duration.is_zero())
            .unwrap_or(READ_DEADLINE),
        FineTuningTimeout::Structured(_) => READ_DEADLINE,
    }
}

// the openai client reads type, param and code off the body's error object
fn sdk_error(body: &[u8], status: u16) -> Result<SdkError, ReadWireError> {
    let value: Value = serde_json::from_slice(body).map_err(ReadWireError::Body)?;
    let error = value.get("error");
    Ok(SdkError {
        status,
        error_type: error_field(error, "type"),
        param: error_field(error, "param"),
        code: error_field(error, "code"),
        message: error_field(error, "message").unwrap_or_default(),
    })
}

fn error_field(error: Option<&Value>, name: &str) -> Option<String> {
    error
        .and_then(|error| error.get(name))
        .and_then(Value::as_str)
        .map(str::to_owned)
}
