use std::collections::BTreeMap;
use std::future::ready;
use std::sync::{Arc, Mutex, PoisonError};

use aws_credential_types::Credentials;
use betterlite_secret_managers::aws::request_signing::{
    AwsCredentialsInfo, AwsOptionalParams, AwsTransport, CredentialResolver, TransportError,
    TransportFuture, TransportRequest, TransportResponse,
};
use betterlite_secret_managers::aws_secret_reads::AwsBlockingTransport;
use http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;

const DEFAULT_REGION: &str = "us-west-2";

pub(super) type Requests = Arc<Mutex<Vec<Value>>>;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Route {
    pub(super) route: String,
    pub(super) replies: Vec<Reply>,
}

#[derive(Clone, Deserialize)]
#[serde(untagged)]
pub(super) enum Reply {
    Answer { status: u16, body: String },
    Timeout { timeout: bool },
    Failure { failure: bool },
}

#[derive(Debug, Error)]
#[error("the connection failed")]
pub(super) struct Refused;

#[derive(Debug, Error)]
pub(super) enum CredentialFailure {
    #[error("AWS_ACCESS_KEY_ID is not set")]
    AccessKey,
    #[error("AWS_SECRET_ACCESS_KEY is not set")]
    SecretKey,
}

pub(super) struct Resolver {
    environment: BTreeMap<String, String>,
}

impl Resolver {
    pub(super) fn new(environment: &BTreeMap<String, String>) -> Self {
        Self {
            environment: environment.clone(),
        }
    }
}

impl CredentialResolver for Resolver {
    type Error = CredentialFailure;

    fn resolve_credentials(
        &self,
        optional_params: &mut AwsOptionalParams,
    ) -> Result<AwsCredentialsInfo, CredentialFailure> {
        let access_key = self
            .environment
            .get("AWS_ACCESS_KEY_ID")
            .ok_or(CredentialFailure::AccessKey)?;
        let secret_key = self
            .environment
            .get("AWS_SECRET_ACCESS_KEY")
            .ok_or(CredentialFailure::SecretKey)?;
        let region = optional_params
            .aws_region_name
            .clone()
            .filter(|region| !region.is_empty())
            .or_else(|| self.environment.get("AWS_REGION_NAME").cloned())
            .or_else(|| self.environment.get("AWS_REGION").cloned())
            .unwrap_or_else(|| DEFAULT_REGION.to_owned());
        Ok(AwsCredentialsInfo {
            aws_region_name: region,
            aws_bedrock_runtime_endpoint: None,
            credentials: Credentials::new(
                access_key.as_str(),
                secret_key.as_str(),
                None,
                None,
                "oracle",
            ),
        })
    }
}

pub(super) struct Script {
    replies: Mutex<BTreeMap<String, Vec<Reply>>>,
    requests: Requests,
}

impl Script {
    pub(super) fn new(routes: &[Route], requests: Requests) -> Self {
        let replies = routes
            .iter()
            .map(|route| (route.route.clone(), route.replies.clone()))
            .collect();
        Self {
            replies: Mutex::new(replies),
            requests,
        }
    }

    fn answer(
        &self,
        request: &TransportRequest<'_>,
        blocking: bool,
    ) -> Result<TransportResponse, TransportError<Refused>> {
        let request_body = serde_json::from_slice::<Value>(request.body).unwrap_or(Value::Null);
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({
                "url": request.url,
                "target": request.headers.get("x-amz-target").and_then(|value| value.to_str().ok()),
                "body": request_body,
                "blocking": blocking,
            }));
        match self.next_reply(&format!("POST {}", request.url)) {
            Some(Reply::Answer {
                status,
                body: reply_body,
            }) => Ok(TransportResponse {
                status: StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                body: reply_body.into_bytes(),
            }),
            Some(Reply::Timeout { timeout: true }) => Err(TransportError::Timeout),
            Some(Reply::Failure { failure: true }) => Err(TransportError::Failed(Refused)),
            Some(Reply::Timeout { timeout: false } | Reply::Failure { failure: false }) | None => {
                Ok(TransportResponse {
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    body: Vec::new(),
                })
            }
        }
    }

    fn next_reply(&self, route: &str) -> Option<Reply> {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(route)
            .and_then(|replies| {
                if replies.len() > 1 {
                    Some(replies.remove(0))
                } else {
                    replies.first().cloned()
                }
            })
    }
}

impl AwsTransport for Script {
    type Error = Refused;

    fn post<'request>(
        &'request self,
        request: TransportRequest<'request>,
    ) -> TransportFuture<'request, Refused> {
        Box::pin(ready(self.answer(&request, false)))
    }
}

impl AwsBlockingTransport for Script {
    fn post_blocking(
        &self,
        request: TransportRequest<'_>,
    ) -> Result<TransportResponse, TransportError<Refused>> {
        self.answer(&request, true)
    }
}
