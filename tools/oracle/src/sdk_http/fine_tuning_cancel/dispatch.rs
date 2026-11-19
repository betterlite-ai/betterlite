use std::string::FromUtf8Error;
use std::time::Duration;

use betterlite_core::api::fine_tuning_cancel_call::{
    AzureFineTuningCancel, FineTuningCancelDispatch, FineTuningCancelTarget, OpenAiFineTuningCancel,
};
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::fine_tuning::error::FineTuneError;
use betterlite_openai::fine_tuning::job::{FineTuneJob, decode_fine_tune_job};
use http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use http::{HeaderMap, HeaderValue, Method};
use secrecy::ExposeSecret as _;
use thiserror::Error;
use url::Url;

const DEADLINE: Duration = Duration::from_secs(30);
const JSON_CONTENT: &str = "application/json";
// the openai client sends the organization it was built with as a header
const ORGANIZATION: &str = "openai-organization";

// the failure a dispatch reports as the call's own, before the outcome names its class
#[derive(Debug, Error)]
pub(crate) enum CancelFailure {
    #[error("the api key is not set")]
    MissingKey,
    #[error("the api base is not set")]
    MissingBase,
    #[error("the cancel call needs a credential a request header cannot carry")]
    InvalidHeader,
    #[error("the provider answered with status {status}")]
    Status { status: u16, body: String },
    #[error("the provider answered with status {status} and a body that is not UTF-8")]
    StatusUtf8 {
        status: u16,
        #[source]
        source: FromUtf8Error,
    },
    #[error(transparent)]
    Url(#[from] url::ParseError),
    #[error(transparent)]
    Transport(#[from] Box<TransportError>),
    #[error(transparent)]
    Decode(#[from] FineTuneError),
}

pub(crate) struct HttpDispatch {
    pub(crate) client: ProxiedClient,
}

impl FineTuningCancelDispatch for HttpDispatch {
    type Error = CancelFailure;

    async fn cancel(
        &self,
        target: FineTuningCancelTarget<'_>,
    ) -> Result<FineTuneJob, CancelFailure> {
        let (request, is_azure) = match &target {
            FineTuningCancelTarget::OpenAi(openai) => (openai_request(openai)?, false),
            FineTuningCancelTarget::Azure(azure) => (azure_request(azure)?, true),
        };
        let response = self
            .client
            .send_buffered(request)
            .await
            .map_err(|source| CancelFailure::Transport(Box::new(source)))?;
        if !(200..300).contains(&response.status) {
            let body =
                String::from_utf8(response.body).map_err(|source| CancelFailure::StatusUtf8 {
                    status: response.status,
                    source,
                })?;
            return Err(CancelFailure::Status {
                status: response.status,
                body,
            });
        }
        decode_fine_tune_job(&response.body, is_azure).map_err(CancelFailure::Decode)
    }
}

fn openai_request(target: &OpenAiFineTuningCancel<'_>) -> Result<OutboundRequest, CancelFailure> {
    let Some(api_key) = target.api_key else {
        return Err(CancelFailure::MissingKey);
    };
    let mut headers = json_headers();
    let authorization = format!("Bearer {}", api_key.expose_secret());
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&authorization).map_err(|_source| CancelFailure::InvalidHeader)?,
    );
    if let Some(organization) = target.organization {
        headers.insert(
            ORGANIZATION,
            HeaderValue::from_str(organization).map_err(|_source| CancelFailure::InvalidHeader)?,
        );
    }
    let url = format!(
        "{}/fine_tuning/jobs/{}/cancel",
        target.api_base.trim_end_matches('/'),
        target.values.job_id
    );
    Ok(OutboundRequest {
        method: Method::POST,
        url: Url::parse(&url)?,
        query: Vec::new(),
        headers,
        body: RequestBody::Empty,
        deadline: DEADLINE,
    })
}

fn azure_request(target: &AzureFineTuningCancel<'_>) -> Result<OutboundRequest, CancelFailure> {
    let Some(api_base) = target.api_base else {
        return Err(CancelFailure::MissingBase);
    };
    let Some(api_key) = target.api_key else {
        return Err(CancelFailure::MissingKey);
    };
    let mut headers = json_headers();
    headers.insert(
        "api-key",
        HeaderValue::from_str(api_key.expose_secret())
            .map_err(|_source| CancelFailure::InvalidHeader)?,
    );
    let url = format!(
        "{}/openai/fine_tuning/jobs/{}/cancel",
        api_base.trim_end_matches('/'),
        target.values.job_id
    );
    let query = target
        .api_version
        .map(|version| vec![("api-version".to_owned(), version.to_owned())])
        .unwrap_or_default();
    Ok(OutboundRequest {
        method: Method::POST,
        url: Url::parse(&url)?,
        query,
        headers,
        body: RequestBody::Empty,
        deadline: DEADLINE,
    })
}

fn json_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(JSON_CONTENT));
    headers.insert(ACCEPT, HeaderValue::from_static(JSON_CONTENT));
    headers
}
