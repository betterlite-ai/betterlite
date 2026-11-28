use std::collections::BTreeMap;
use std::fmt::Display;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::Section as _;
use betterlite_config::sections::openai_credentials::{
    Raw as OpenAiCredentialsRaw, Settings as OpenAiCredentialsSettings,
};
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::container_collection_http::{
    CollectionReply, CollectionRequest, CollectionTransport,
};
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::pool::PoolError;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_logging::logging_pre_post::args::AdditionalArgs;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::pre_call::PreCall;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_openai::containers::OpenAiContainerConfig;
use betterlite_openai::containers::container_file_http::PreCallHook;
use betterlite_openai::containers::container_file_http::request::SslVerify;
use betterlite_openai::containers::container_file_http::transport::{
    ClientSource, ContainerHttpReply, TransferError,
};
use betterlite_openai::containers::costs::ContainerCosts;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use http::Method;
use http::header::HeaderMap;
use rust_decimal::Decimal;
use secrecy::SecretString;
use tokio::io::AsyncReadExt as _;
use tokio::net::TcpListener;
use tokio::runtime::Runtime;
use tokio::time::sleep;
use url::Url;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};

const INIT: &str = r#"{"model":"openai/container","call_type":"create_container","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const READ_TIMEOUT: Duration = Duration::from_millis(100);
const HOLD: Duration = Duration::from_millis(300);

// B068 owns the real arithmetic; the create reply's cost stays out of the compared record
#[derive(Debug)]
struct FlatCost;

impl ContainerCosts for FlatCost {
    fn code_interpreter_cost(&self, _sessions: u32, _provider: &str) -> Decimal {
        Decimal::ZERO
    }
}

pub(super) fn config(
    environment: &BTreeMap<String, String>,
) -> Result<OpenAiContainerConfig, Box<HelperError>> {
    let settings: OpenAiCredentialsSettings = OpenAiCredentialsRaw::default()
        .convert(environment)
        .or_config_error()?;
    let env_api_key = environment
        .get("OPENAI_API_KEY")
        .map(|key| SecretString::from(key.as_str()));
    Ok(OpenAiContainerConfig {
        settings,
        globals: LiteLlmGlobals::default(),
        env_api_key,
        costs: Arc::new(FlatCost),
    })
}

pub(super) fn logging_state() -> Result<LoggingState, Box<HelperError>> {
    let init: LoggingInit = serde_json::from_str(INIT).or_value_error()?;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_config_error()?;
    let mut context = CorrelationContext::default();
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("generated"),
        Duration::default,
    )
    .or_config_error()
}

#[derive(Default)]
pub(super) struct RecordingHook {
    calls: Mutex<Vec<(Json, AdditionalArgs)>>,
}

impl RecordingHook {
    pub(super) fn calls(&self) -> Vec<(Json, AdditionalArgs)> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl PreCallHook for RecordingHook {
    fn pre_call(&self, _state: &mut LoggingState, call: &PreCall) -> Result<(), PrePostError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((call.input.clone(), call.additional_args.clone()));
        Ok(())
    }
}

struct Inner {
    requests: Mutex<Vec<CollectionRequest>>,
    reply: ContainerHttpReply,
    failure: Mutex<Option<TransferError>>,
}

// Answers with the case's reply, or with one transport failure, and keeps every request it was handed
#[derive(Clone)]
pub(super) struct Fake(Arc<Inner>);

impl Fake {
    pub(super) fn new(reply: ContainerHttpReply, stopped: Option<TransferError>) -> Self {
        Self(Arc::new(Inner {
            requests: Mutex::default(),
            reply,
            failure: Mutex::new(stopped),
        }))
    }

    fn answer(&self, request: &CollectionRequest) -> CollectionReply {
        self.0
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        let stopped = self
            .0
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        stopped.map_or_else(|| Ok(self.0.reply.clone()), Err)
    }

    pub(super) fn requests(&self) -> Vec<CollectionRequest> {
        self.0
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CollectionTransport for Fake {
    fn send(&self, request: &CollectionRequest) -> impl Future<Output = CollectionReply> + Send {
        ready(self.answer(request))
    }
}

impl ClientSource for Fake {
    type Blocking = Self;
    type Asynchronous = Self;

    fn blocking_client(
        &self,
        _ssl_verify: Option<&SslVerify>,
    ) -> Result<Self::Blocking, PoolError> {
        Ok(self.clone())
    }

    fn async_client(
        &self,
        _ssl_verify: Option<&SslVerify>,
    ) -> Result<Self::Asynchronous, PoolError> {
        Ok(self.clone())
    }
}

// A local listener holds its answer past a 100ms read timeout, so the transport raises a real deadline failure
pub(super) fn deadline(runtime: &Runtime) -> Result<TransferError, Box<HelperError>> {
    let policy =
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).or_config_error()?;
    let client = ProxiedClient::build(
        base_builder(),
        &Arc::new(policy),
        TransportLimits {
            read_timeout: READ_TIMEOUT,
            ..TransportLimits::DEFAULT
        },
    )
    .or_config_error()?;
    runtime.block_on(send_into_the_hold(&client))
}

// The listener reads the one request, then holds past the read timeout, so the send ends in a deadline failure
async fn send_into_the_hold(client: &ProxiedClient) -> Result<TransferError, Box<HelperError>> {
    let listener = TcpListener::bind("127.0.0.1:0").await.or_config_error()?;
    let address = listener.local_addr().or_config_error()?;
    let url = Url::parse(&format!("http://{address}/v1/containers")).or_value_error()?;
    let request = OutboundRequest {
        method: Method::GET,
        url,
        query: Vec::new(),
        headers: HeaderMap::new(),
        body: RequestBody::Empty,
        deadline: HOLD,
    };
    let held = async {
        let (mut stream, _) = listener.accept().await.or_config_error()?;
        let mut chunk = [0_u8; 64];
        let read = stream.read(&mut chunk).await.or_config_error()?;
        if read == 0 {
            return Err(failure(&"the client sent no request"));
        }
        sleep(HOLD).await;
        Ok(())
    };
    let (result, listener_finished) = tokio::join!(client.send_buffered(request), held);
    listener_finished?;
    match result {
        Err(stopped) => Ok(TransferError::Transport(stopped)),
        Ok(_) => Err(failure(
            &"the local listener answered instead of timing out",
        )),
    }
}

pub(super) fn failure(error: &dyn Display) -> Box<HelperError> {
    Box::new(HelperError {
        class: "OSError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
