use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::workload_identity::config::SdkCapability;
use betterlite_openai::workload_identity::error::WorkloadIdentityError;
use betterlite_openai::workload_identity::transport::WorkloadIdentityTransport;
use http::HeaderMap;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::task;

const EXHAUSTED_STATUS: u16 = 500;

#[derive(Clone, Deserialize)]
#[serde(untagged)]
pub(super) enum Reply {
    Failure { failure: bool },
    Answer { status: u16, body: String },
}

pub(super) struct Script {
    replies: Mutex<VecDeque<Reply>>,
    pub(super) requests: Mutex<Vec<Value>>,
}

impl Script {
    pub(super) fn new(replies: &[Reply]) -> Self {
        Self {
            replies: Mutex::new(replies.iter().cloned().collect()),
            requests: Mutex::default(),
        }
    }

    fn answer(&self, request: &OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        let json_body = match &request.body {
            RequestBody::Json(data) => serde_json::from_slice(data).unwrap_or(Value::Null),
            RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
                Value::Null
            }
        };
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({
                "method": request.method.as_str(),
                "url": request.url.to_string(),
                "json": json_body,
                "timeout_ms": request.deadline.as_millis(),
            }));
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match reply {
            Some(Reply::Failure { failure: true }) => Err(TransportError::Closed),
            Some(Reply::Answer { status, body }) => Ok(UpstreamResponse {
                status,
                headers: HeaderMap::new(),
                body: body.into_bytes(),
            }),
            Some(Reply::Failure { failure: false }) | None => Ok(UpstreamResponse {
                status: EXHAUSTED_STATUS,
                headers: HeaderMap::new(),
                body: Vec::new(),
            }),
        }
    }
}

impl WorkloadIdentityTransport for Script {
    // yields once so a second caller is polled while the first is mid-exchange
    async fn send_buffered(
        &self,
        request: OutboundRequest,
    ) -> Result<UpstreamResponse, TransportError> {
        task::yield_now().await;
        self.answer(&request)
    }
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Sdk {
    #[default]
    Available,
    NoAuthClass,
    NoK8sProvider,
    None,
}

impl Sdk {
    pub(super) const fn capability(self) -> SdkCapability {
        SdkCapability {
            workload_identity_auth: matches!(self, Self::Available | Self::NoK8sProvider),
            k8s_token_provider: matches!(self, Self::Available | Self::NoAuthClass),
        }
    }
}

pub(super) fn error_json(error: &WorkloadIdentityError, directory: &str) -> Value {
    match error {
        WorkloadIdentityError::SdkUnavailable { provider, .. } => {
            json!({"kind": "sdk_unavailable", "status": provider.status_code, "message": provider.message})
        }
        WorkloadIdentityError::TokenFile { .. } => {
            json!({"kind": "subject_token", "message": error.to_string().replace(directory, "<dir>")})
        }
        WorkloadIdentityError::Transport(_) => json!({"kind": "transport"}),
        WorkloadIdentityError::OAuth {
            status,
            error: code,
            message,
        } => json!({"kind": "oauth", "status": status, "error": code, "message": message}),
        WorkloadIdentityError::EmptyBody
        | WorkloadIdentityError::InvalidField { .. }
        | WorkloadIdentityError::ExchangeStatus { .. } => {
            json!({"kind": "openai", "message": error.to_string()})
        }
        WorkloadIdentityError::TokenUnusable => {
            json!({"kind": "runtime", "message": error.to_string()})
        }
        WorkloadIdentityError::ApiBase(_)
        | WorkloadIdentityError::ExchangeUrl(_)
        | WorkloadIdentityError::Encode(_) => json!({"kind": "internal"}),
    }
}
