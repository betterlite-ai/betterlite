use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_http::error::HttpError;
use betterlite_http::proxy::pool::PoolError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::pre_call::PreCall;
use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::containers::container_file_http::PreCallHook;
use betterlite_openai::containers::container_file_http::request::SslVerify;
use betterlite_openai::containers::container_file_http::transport::{
    AsyncClient, BlockingClient, ClientSource, ContainerHttpReply, ContainerHttpRequest,
    TransferError,
};
use serde_json::{Map, Value, json};

use super::{Failure, ReplyInput};
use betterlite_oracle_helper_api::error::HelperError;

#[derive(Clone)]
pub(super) struct ScriptedSource {
    client: ScriptedClient,
    requests: Arc<Mutex<Vec<ContainerHttpRequest>>>,
}

impl ScriptedSource {
    pub(super) fn new(reply: &ReplyInput, failure: Failure) -> Self {
        let requests = Arc::new(Mutex::new(Vec::new()));
        Self {
            client: ScriptedClient {
                reply: ContainerHttpReply {
                    status: reply.status,
                    headers: reply.headers.clone(),
                    body: reply.body.clone().into_bytes(),
                },
                failure,
                requests: Arc::clone(&requests),
            },
            requests,
        }
    }

    pub(super) fn only_request(&self) -> Result<ContainerHttpRequest, Box<HelperError>> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .first()
            .cloned()
            .ok_or_else(missing_request)
    }
}

impl ClientSource for ScriptedSource {
    type Blocking = ScriptedClient;
    type Asynchronous = ScriptedClient;

    fn blocking_client(
        &self,
        _ssl_verify: Option<&SslVerify>,
    ) -> Result<ScriptedClient, PoolError> {
        Ok(self.client.clone())
    }

    fn async_client(&self, _ssl_verify: Option<&SslVerify>) -> Result<ScriptedClient, PoolError> {
        Ok(self.client.clone())
    }
}

#[derive(Clone)]
pub(super) struct ScriptedClient {
    reply: ContainerHttpReply,
    failure: Failure,
    requests: Arc<Mutex<Vec<ContainerHttpRequest>>>,
}

impl ScriptedClient {
    fn answer(&self) -> Result<ContainerHttpReply, TransferError> {
        match self.failure {
            Failure::Answer => Ok(self.reply.clone()),
            Failure::Timeout => Err(TransferError::from(TransportError::Rejected(
                HttpError::Timeout,
            ))),
        }
    }

    fn record(&self, request: &ContainerHttpRequest) {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
    }
}

impl BlockingClient for ScriptedClient {
    fn send(&self, request: &ContainerHttpRequest) -> Result<ContainerHttpReply, TransferError> {
        self.record(request);
        self.answer()
    }
}

impl AsyncClient for ScriptedClient {
    fn send(
        &self,
        request: &ContainerHttpRequest,
    ) -> impl Future<Output = Result<ContainerHttpReply, TransferError>> + Send {
        self.record(request);
        ready(self.answer())
    }
}

#[derive(Clone, Default)]
pub(super) struct RecordingHook {
    logged: Arc<Mutex<Option<Value>>>,
}

impl RecordingHook {
    pub(super) fn record(&self) -> Option<Value> {
        self.logged
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl PreCallHook for RecordingHook {
    fn pre_call(&self, _state: &mut LoggingState, record: &PreCall) -> Result<(), PrePostError> {
        *self.logged.lock().unwrap_or_else(PoisonError::into_inner) = Some(logged_json(record));
        Ok(())
    }
}

fn logged_json(record: &PreCall) -> Value {
    let arguments = &record.additional_args;
    let headers: Map<String, Value> = arguments
        .headers
        .iter()
        .flatten()
        .map(|(name, value)| (name.to_lowercase(), json!(value)))
        .collect();
    json!({
        "api_base": arguments.api_base,
        "headers": headers,
        "params": arguments.passthrough.get("params"),
        "container_id": arguments.passthrough.get("container_id"),
    })
}

fn missing_request() -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("ValueError"),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: String::from("the handler sent no request"),
    })
}
