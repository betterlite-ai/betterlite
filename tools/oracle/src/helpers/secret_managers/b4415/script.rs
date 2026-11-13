use std::collections::{BTreeMap, VecDeque};
use std::future::ready;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use aws_credential_types::Credentials;
use betterlite_secret_managers::aws::request_signing::{
    AwsCredentialsInfo, AwsOptionalParams, AwsTransport, CredentialResolver, TransportError,
    TransportFuture, TransportRequest, TransportResponse,
};
use betterlite_secret_managers::aws_secret_reads::blocking::AwsBlockingTransport;
use http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

const DEFAULT_REGION: &str = "us-west-2";

pub(super) type Reply = Result<TransportResponse, TransportError<io::Error>>;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Route {
    pub(super) route: String,
    pub(super) replies: Vec<ScriptedReply>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ScriptedReply {
    status: Option<u16>,
    body: Option<String>,
    #[serde(default)]
    timeout: bool,
    #[serde(default)]
    failure: bool,
}

#[derive(Default)]
struct State {
    queues: BTreeMap<String, VecDeque<ScriptedReply>>,
    tokens: BTreeMap<String, String>,
    seen: Vec<Value>,
}

pub(super) struct Script {
    state: Mutex<State>,
}

impl Script {
    pub(super) fn new(routes: &[Route]) -> Arc<Self> {
        let queues = routes
            .iter()
            .map(|route| (route.route.clone(), VecDeque::from(route.replies.clone())))
            .collect();
        Arc::new(Self {
            state: Mutex::new(State {
                queues,
                tokens: BTreeMap::new(),
                seen: Vec::new(),
            }),
        })
    }

    pub(super) fn take_requests(&self) -> Vec<Value> {
        std::mem::take(
            &mut self
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .seen,
        )
    }

    fn answer(&self, request: &TransportRequest<'_>) -> Reply {
        let mut body = serde_json::from_slice::<Value>(request.body).unwrap_or(Value::Null);
        let target = request
            .headers
            .get("x-amz-target")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let scripted = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.normalize_token(&mut body);
            state.seen.push(json!({
                "url": request.url,
                "target": target,
                "body": body,
            }));
            let route = format!("POST {}", request.url);
            state.queues.get_mut(&route).and_then(|queue| {
                if queue.len() > 1 {
                    queue.pop_front()
                } else {
                    queue.front().cloned()
                }
            })
        };
        reply_from(scripted)
    }
}

fn reply_from(scripted: Option<ScriptedReply>) -> Reply {
    let Some(reply) = scripted else {
        return Ok(TransportResponse {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            body: Vec::new(),
        });
    };
    if reply.timeout {
        return Err(TransportError::Timeout);
    }
    if reply.failure {
        return Err(TransportError::Failed(io::Error::from(
            io::ErrorKind::ConnectionRefused,
        )));
    }
    let status = reply.status.unwrap_or(500);
    Ok(TransportResponse {
        status: StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        body: reply.body.unwrap_or_default().into_bytes(),
    })
}

impl State {
    fn normalize_token(&mut self, body: &mut Value) {
        let Some(fields) = body.as_object_mut() else {
            return;
        };
        let found = fields
            .get("ClientRequestToken")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let Some(token) = found else {
            return;
        };
        if Uuid::parse_str(&token).is_err() {
            return;
        }
        let next = format!("<uuid-{}>", self.tokens.len().saturating_add(1));
        let placeholder = self.tokens.entry(token).or_insert(next).clone();
        fields.insert("ClientRequestToken".to_owned(), json!(placeholder));
    }
}

impl AwsTransport for Script {
    type Error = io::Error;

    fn post<'request>(
        &'request self,
        request: TransportRequest<'request>,
    ) -> TransportFuture<'request, io::Error> {
        Box::pin(ready(self.answer(&request)))
    }
}

impl AwsBlockingTransport for Script {
    fn post_blocking(&self, request: TransportRequest<'_>) -> Reply {
        self.answer(&request)
    }
}

pub(super) struct Resolver {
    pub(super) environment: BTreeMap<String, String>,
}

impl CredentialResolver for Resolver {
    type Error = io::Error;

    fn resolve_credentials(
        &self,
        optional_params: &mut AwsOptionalParams,
    ) -> Result<AwsCredentialsInfo, io::Error> {
        let missing = |name: &str| io::Error::new(io::ErrorKind::NotFound, name.to_owned());
        let access_key = self
            .environment
            .get("AWS_ACCESS_KEY_ID")
            .ok_or_else(|| missing("AWS_ACCESS_KEY_ID"))?;
        let secret_key = self
            .environment
            .get("AWS_SECRET_ACCESS_KEY")
            .ok_or_else(|| missing("AWS_SECRET_ACCESS_KEY"))?;
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
            credentials: Credentials::new(access_key, secret_key, None, None, "oracle"),
        })
    }
}
