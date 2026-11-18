use std::collections::BTreeMap;
use std::future::ready;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::env_integer::timeout::RequestTimeout;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_config::types::ProxyTrust;
use betterlite_http::interactions::base::clients::HttpClients;
use betterlite_http::interactions::base::{
    ErrorClasses, Logging, Params, PreparedRequest, RequestHeaders, ResponseHeaders,
    UpstreamResponse,
};
use betterlite_http::interactions::get::{EnvironmentRequest, InteractionIdRequest};
use betterlite_http::interactions::lifecycle::InteractionLifecycleConfig;
use betterlite_interactions::polling::context::{InteractionPollContext, SettlementFlag};
use betterlite_interactions::polling::host::{
    FetchFuture, FetchInteraction, SettlementFuture, SettlementHost,
};
use betterlite_interactions::polling::registry::BackgroundPollRegistry;
use betterlite_interactions::types::response::{
    CancelInteractionResult, DeleteInteractionResult, InteractionsApiResponse,
};
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;

use crate::api::agents::item::error::ProviderFailure;
use crate::api::interactions::lifecycle::InteractionLifecycleDeps;
use crate::api::interactions::lifecycle::request::InteractionLifecycleRequest;

// the provider config's own error: the status the upstream answered with and the body it carried
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(crate) struct FixtureError {
    pub(crate) status: u16,
    pub(crate) message: String,
}

impl From<serde_json::Error> for FixtureError {
    fn from(error: serde_json::Error) -> Self {
        Self {
            status: 500,
            message: error.to_string(),
        }
    }
}

impl ProviderFailure for FixtureError {
    fn status_code(&self) -> u16 {
        self.status
    }

    fn message(&self) -> String {
        self.message.clone()
    }
}

pub(crate) struct Config {
    params: Mutex<Option<Params>>,
}

#[must_use]
pub(crate) fn fixture() -> Config {
    Config {
        params: Mutex::new(None),
    }
}

impl Config {
    pub(crate) fn params(&self) -> Params {
        self.params
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .expect("the environment was validated")
    }
}

impl ErrorClasses for Config {
    type Error = FixtureError;

    fn get_error_class(
        &self,
        message: &str,
        status_code: u16,
        _headers: &ResponseHeaders,
    ) -> FixtureError {
        FixtureError {
            status: status_code,
            message: message.to_owned(),
        }
    }
}

impl InteractionLifecycleConfig for Config {
    type DeleteOutput = DeleteInteractionResult;
    type CancelOutput = CancelInteractionResult;

    fn validate_environment(
        &self,
        request: &EnvironmentRequest<'_>,
    ) -> Result<RequestHeaders, FixtureError> {
        *self.params.lock().unwrap_or_else(PoisonError::into_inner) =
            request.litellm_params.cloned();
        Ok(request.headers.clone())
    }

    fn transform_delete_request(
        &self,
        request: &InteractionIdRequest<'_>,
    ) -> Result<PreparedRequest, FixtureError> {
        Ok(prepared_request(request, ""))
    }

    fn transform_delete_response(
        &self,
        _response: &UpstreamResponse,
        _logging: &mut Logging,
        interaction_id: &str,
    ) -> Result<DeleteInteractionResult, FixtureError> {
        Ok(serde_json::from_str(&format!(
            r#"{{"success":true,"id":"{interaction_id}"}}"#
        ))?)
    }

    fn transform_cancel_request(
        &self,
        request: &InteractionIdRequest<'_>,
    ) -> Result<PreparedRequest, FixtureError> {
        let mut cancel = prepared_request(request, ":cancel");
        cancel
            .params
            .insert("state".to_owned(), JsonValue::Text("cancelled".to_owned()));
        Ok(cancel)
    }

    fn transform_cancel_response(
        &self,
        _response: &UpstreamResponse,
        _logging: &mut Logging,
    ) -> Result<CancelInteractionResult, FixtureError> {
        Ok(serde_json::from_str(
            r#"{"id":"int-1","status":"cancelled"}"#,
        )?)
    }
}

fn prepared_request(request: &InteractionIdRequest<'_>, suffix: &str) -> PreparedRequest {
    PreparedRequest {
        url: format!(
            "{}/v1beta/interactions/{}{suffix}",
            request.api_base, request.interaction_id
        ),
        params: Params::new(),
    }
}

pub(crate) fn deps<'deps>(
    http: &'deps mut Logging,
    state: &'deps mut LoggingState,
    redactor: &'deps Redactor,
) -> InteractionLifecycleDeps<'deps> {
    InteractionLifecycleDeps {
        clients: HttpClients::new(policy(), proxy()),
        http,
        logging: state,
        redactor,
    }
}

pub(crate) fn state(model: &str, call_id: &str) -> LoggingState {
    let mut context = CorrelationContext::default();
    let init: LoggingInit = serde_json::from_str(&format!(
        r#"{{"model":"{model}","call_type":"generated","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"{call_id}","function_id":""}}"#
    ))
    .expect("the logging fixture parses");
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &redactor(),
        || String::from("generated-trace"),
        Duration::default,
    )
    .expect("the logging fixture builds")
}

pub(crate) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles")
}

fn policy() -> TlsPolicy {
    TlsPolicy {
        verification: Verification::Enabled,
        ca_file: None,
        certificate: None,
        http2: false,
        ecdh_curve: None,
        user_agent: "betterlite-test/0".to_owned(),
    }
}

fn proxy() -> Arc<ProxyPolicy> {
    let environment: BTreeMap<String, String> = BTreeMap::new();
    Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &environment).unwrap())
}

pub(crate) struct Call<'call> {
    interaction_id: &'call str,
    provider: Option<&'call str>,
    headers: BTreeMap<String, String>,
    timeout: Option<Duration>,
    seconds: f64,
    kwargs: IndexMap<String, Json>,
}

impl<'call> Call<'call> {
    #[must_use]
    pub(crate) fn new(interaction_id: &'call str) -> Self {
        Self {
            interaction_id,
            provider: None,
            headers: BTreeMap::new(),
            timeout: None,
            seconds: 6000.0,
            kwargs: IndexMap::new(),
        }
    }

    #[must_use]
    pub(crate) fn provider(mut self, provider: &'call str) -> Self {
        self.provider = Some(provider);
        self
    }

    #[must_use]
    pub(crate) fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.insert(name.to_owned(), value.to_owned());
        self
    }

    #[must_use]
    pub(crate) fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    #[must_use]
    pub(crate) fn seconds(mut self, seconds: f64) -> Self {
        self.seconds = seconds;
        self
    }

    #[must_use]
    pub(crate) fn kwarg(mut self, name: &str, value: Json) -> Self {
        self.kwargs.insert(name.to_owned(), value);
        self
    }

    #[must_use]
    pub(crate) fn at(mut self, address: SocketAddr) -> Self {
        self.kwargs
            .insert("api_base".to_owned(), Json::Text(base(address)));
        self
    }

    #[must_use]
    pub(crate) fn request(&self) -> InteractionLifecycleRequest<'_> {
        InteractionLifecycleRequest {
            interaction_id: self.interaction_id,
            extra_headers: self.headers.clone(),
            timeout: self.timeout,
            custom_llm_provider: self.provider,
            request_timeout: RequestTimeout {
                seconds: self.seconds,
                explicitly_set: false,
            },
            kwargs: &self.kwargs,
        }
    }
}

pub(crate) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(crate) fn base(address: SocketAddr) -> String {
    format!("http://{address}")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub(crate) struct FakeError(pub(crate) &'static str);

#[derive(Clone)]
pub(crate) struct FakeHost {
    billed: Arc<Mutex<Vec<String>>>,
    billing_fails: bool,
}

impl FakeHost {
    pub(crate) fn billed(&self) -> Vec<String> {
        self.billed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    #[must_use]
    pub(crate) fn failing_billing(mut self) -> Self {
        self.billing_fails = true;
        self
    }
}

impl SettlementHost for FakeHost {
    type Reservation = String;
    type BillingError = FakeError;
    type ReleaseError = FakeError;

    fn budget_reservation(&self) -> Option<String> {
        None
    }

    fn log_background_completion<'host>(
        &'host self,
        response: &'host InteractionsApiResponse,
    ) -> SettlementFuture<'host, Result<(), FakeError>> {
        let billed = Arc::clone(&self.billed);
        let interaction_id = response.id.clone().unwrap_or_default();
        let fails = self.billing_fails;
        Box::pin(async move {
            billed
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(interaction_id);
            if fails {
                return Err(FakeError("billing failed"));
            }
            Ok(())
        })
    }

    fn release_reservation<'host>(
        &'host self,
        _reservation: &'host String,
    ) -> SettlementFuture<'host, Result<(), FakeError>> {
        Box::pin(ready(Ok(())))
    }
}

pub(crate) fn host() -> FakeHost {
    FakeHost {
        billed: Arc::default(),
        billing_fails: false,
    }
}

#[derive(Clone)]
pub(crate) struct Script {
    answer: Arc<Mutex<Option<Result<InteractionsApiResponse, FakeError>>>>,
    calls: Arc<AtomicUsize>,
}

impl Script {
    pub(crate) fn returning(answer: InteractionsApiResponse) -> Self {
        Self {
            answer: Arc::new(Mutex::new(Some(Ok(answer)))),
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub(crate) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    pub(crate) fn fetch(&self) -> FetchInteraction<FakeHost, FakeError> {
        let answer = Arc::clone(&self.answer);
        let calls = Arc::clone(&self.calls);
        Arc::new(move |_context| -> FetchFuture<FakeError> {
            calls.fetch_add(1, Ordering::SeqCst);
            let scripted = answer
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take()
                .unwrap_or(Err(FakeError("no scripted answer")));
            Box::pin(async move { scripted })
        })
    }
}

pub(crate) fn answer() -> InteractionsApiResponse {
    serde_json::from_str(
        r#"{"id":"interaction-1","status":"completed","usage":{"total_tokens":5}}"#,
    )
    .expect("the scripted response deserializes")
}

fn poll_context(host: FakeHost) -> InteractionPollContext<FakeHost> {
    InteractionPollContext {
        interaction_id: "interaction-1".to_owned(),
        custom_llm_provider: "gemini".to_owned(),
        host,
        api_key: None,
        api_base: None,
        initial_interval: Duration::from_secs(3600),
        max_interval: Duration::from_secs(3600),
        timeout: Duration::from_secs(3600),
        settlement: SettlementFlag::default(),
    }
}

// an hour-long first interval keeps the spawned poll asleep for the whole test
pub(crate) fn registry_with(host: FakeHost) -> BackgroundPollRegistry<FakeHost> {
    let mut registry = BackgroundPollRegistry::default();
    let poll_fetch: FetchInteraction<FakeHost, FakeError> =
        Arc::new(|_context| Box::pin(ready(Ok(answer()))));
    assert!(registry.schedule(poll_context(host), poll_fetch));
    registry
}
