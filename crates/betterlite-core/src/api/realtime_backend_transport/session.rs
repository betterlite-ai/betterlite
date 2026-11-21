use std::error::Error as StdError;
use std::sync::Arc;

use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_config::tls::Verification;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use tokio_util::sync::CancellationToken;

use crate::api::realtime_backend_relay::source::RealtimeBackendSource;
use crate::api::realtime_contracts::close::{
    close_after_handshake_refusal, realtime_error_event, websocket_close_reason,
};
use crate::api::realtime_contracts::config::{BaseRealtimeConfig, Environment};
use crate::api::realtime_contracts::error::{HeaderMap, WebSocketError};
use crate::api::realtime_contracts::events::AudioTranscriptionUsage;
use crate::api::realtime_contracts::transport::{RealtimeBackend, RealtimeMessage};
use crate::api::realtime_guardrail_policy::RealtimeGuardrailCallback;
use crate::api::realtime_session_state::logging::{RealtimeLogging, RealtimeSuccessQueue};
use crate::api::realtime_session_state::{
    ConnectionSetup, RealtimeProviderConfig, RealtimeStreaming,
};

use super::dial::{
    ConnectedBackend, DialRequest, OPEN_TIMEOUT, RealtimeWebSocketDialer, SizeLimitedSource,
    open_realtime_backend_ws, realtime_ssl,
};
use super::error::{ClientCloseError, DialError, RealtimeConnectionError, is_ignorable_close};
use super::protocol::{
    ClientHandle, ClientSlot, RealtimeSessionRelay, SessionRelay, client_wants_beta,
};

mod setup;

// Everything async_realtime reads for one connection: the provider, the injected websockets client and relay, and B1006's connection inputs
pub struct RealtimeCall<'call, Config, Callback, Relay>
where
    Config: BaseRealtimeConfig + 'static,
    Callback: RealtimeGuardrailCallback + ?Sized,
    Callback::Error: StdError + Send + 'static,
    Relay: RealtimeSessionRelay,
{
    pub model: &'call str,
    pub api_base: Option<&'call str>,
    pub api_key: Option<&'call str>,
    pub headers: &'call HeaderMap,
    pub provider: Arc<Config>,
    pub dialer: &'call dyn RealtimeWebSocketDialer,
    pub client: ClientHandle,
    pub relay: &'call mut Relay,
    pub callbacks: &'call [&'call Callback],
    pub logging: Option<Box<dyn RealtimeLogging>>,
    pub logging_worker: Option<Box<dyn RealtimeSuccessQueue>>,
    pub logging_session_id: &'call str,
    pub user_api_key_dict: Option<Json>,
    pub logged_real_time_event_types: LoggedEventTypes,
    pub litellm_metadata: Option<&'call IndexMap<String, Json>>,
    pub query_params: Option<&'call IndexMap<String, String>>,
    pub verification: Verification,
    pub max_message_size_bytes: usize,
    pub redactor: &'call Redactor,
    pub cancellation: CancellationToken,
}

// B1006's session state reads the two provider-config methods through this adapter, so the provider's associated types stay out of the state
struct ProviderState<Config> {
    provider: Arc<Config>,
}

impl<Config> RealtimeProviderConfig for ProviderState<Config>
where
    Config: BaseRealtimeConfig,
{
    fn requires_session_configuration(&self) -> bool {
        self.provider.requires_session_configuration()
    }

    fn unbilled_usage_on_close(&self, model: &str) -> Option<AudioTranscriptionUsage> {
        self.provider.session_close_usage(model)
    }
}

impl<Config, Callback, Relay> RealtimeCall<'_, Config, Callback, Relay>
where
    Config: BaseRealtimeConfig + 'static,
    Callback: RealtimeGuardrailCallback + ?Sized,
    Callback::Error: StdError + Send + 'static,
    Relay: RealtimeSessionRelay,
{
    // LiteLLM's async_realtime: the URL and headers resolve first, then every failure inside the connection try gets a client close
    pub async fn async_realtime(
        &mut self,
    ) -> Result<(), RealtimeConnectionError<Config::Error, Callback::Error, Relay::Error>> {
        let url = self
            .provider
            .get_complete_url(self.api_base, self.model, self.api_key);
        let environment = Environment {
            headers: self.headers,
            model: self.model,
            api_key: self.api_key,
        };
        let headers = self
            .provider
            .validate_environment(&environment)
            .map_err(RealtimeConnectionError::Config)?;
        match self.run(&url, &headers).await {
            Ok(()) => Ok(()),
            Err(RealtimeConnectionError::Dial(DialError::InvalidStatus {
                status_code, ..
            })) => self.refuse(status_code).await,
            Err(error) => self.handle_failure(&error).await,
        }
    }

    async fn run(
        &mut self,
        url: &str,
        headers: &HeaderMap,
    ) -> Result<(), RealtimeConnectionError<Config::Error, Callback::Error, Relay::Error>> {
        let ssl = realtime_ssl(url, &self.verification);
        let provider_backend = self
            .provider
            .open_backend(url, headers)
            .await
            .map_err(RealtimeConnectionError::Config)?;
        let connected = match provider_backend {
            Some(backend) => ConnectedBackend {
                sender: Box::new(backend),
                source: None,
            },
            None => {
                let request = DialRequest {
                    url,
                    headers,
                    ssl: &ssl,
                    max_size: self.max_message_size_bytes,
                    open_timeout: OPEN_TIMEOUT,
                };
                open_realtime_backend_ws(self.dialer, &request)
                    .await
                    .map_err(RealtimeConnectionError::Dial)?
            }
        };
        let ConnectedBackend { sender, mut source } = connected;
        let mut state = self.build_state(sender);
        let mut limited = source.as_deref_mut().map(|inner| SizeLimitedSource {
            inner,
            max_message_size_bytes: self.max_message_size_bytes,
        });
        let source_ref = limited
            .as_mut()
            .map(|limited| limited as &mut dyn RealtimeBackendSource);
        let setup = self.send_setup(&mut state).await;
        let relayed = match setup {
            Ok(()) => {
                let synthetic = self.send_synthetic(&mut state).await;
                match synthetic {
                    Ok(()) => self
                        .relay
                        .relay(SessionRelay {
                            state: &mut state,
                            source: source_ref,
                            cancellation: &self.cancellation,
                        })
                        .await
                        .map_err(RealtimeConnectionError::Relay),
                    Err(error) => Err(error),
                }
            }
            Err(error) => Err(error),
        };
        // the async context manager exits on every path, and a close failure replaces whatever the body was returning
        let closed = self.close_backend(&mut state).await;
        closed?;
        relayed
    }

    fn build_state(&mut self, backend: Box<dyn RealtimeBackend>) -> RealtimeStreaming {
        let mut request_data = IndexMap::new();
        if let Some(metadata) = self.litellm_metadata {
            if !metadata.is_empty() {
                request_data.insert(
                    "litellm_metadata".to_owned(),
                    Json::Object(metadata.clone()),
                );
            }
        }
        let provider_config: Box<dyn RealtimeProviderConfig> = Box::new(ProviderState {
            provider: Arc::clone(&self.provider),
        });
        let transcription_model = setup::forced_transcription_model(self.model, self.query_params);
        RealtimeStreaming::new(ConnectionSetup {
            client: Some(Box::new(ClientSlot {
                handle: self.client.clone(),
            })),
            backend: Some(backend),
            logging: self.logging.take(),
            logging_worker: self.logging_worker.take(),
            provider_config: Some(provider_config),
            event_normalizer: None,
            logged_real_time_event_types: std::mem::take(&mut self.logged_real_time_event_types),
            model: self.model.to_owned(),
            user_api_key_dict: self.user_api_key_dict.take(),
            request_data: Some(request_data),
            force_transcription_model: transcription_model,
            client_wants_beta: client_wants_beta(&self.client.scope),
            backend_uses_beta_protocol: None,
        })
    }

    async fn send_backend(
        &self,
        state: &mut RealtimeStreaming,
        text: &str,
    ) -> Result<(), RealtimeConnectionError<Config::Error, Callback::Error, Relay::Error>> {
        let Some(backend) = state.backend.as_mut() else {
            return Err(RealtimeConnectionError::Backend(WebSocketError {
                message: "the realtime backend websocket is not connected".to_owned(),
            }));
        };
        backend
            .send(RealtimeMessage::Text(text.to_owned()))
            .await
            .map_err(RealtimeConnectionError::Backend)
    }

    async fn close_backend(
        &self,
        state: &mut RealtimeStreaming,
    ) -> Result<(), RealtimeConnectionError<Config::Error, Callback::Error, Relay::Error>> {
        let Some(mut backend) = state.backend.take() else {
            return Ok(());
        };
        backend
            .close()
            .await
            .map_err(RealtimeConnectionError::Backend)
    }

    // LiteLLM answers a handshake rejection with B1004's error event and the status-mapped close, and any close failure propagates
    async fn refuse(
        &self,
        status_code: u16,
    ) -> Result<(), RealtimeConnectionError<Config::Error, Callback::Error, Relay::Error>> {
        tracing::error!(
            "Error connecting to backend: the realtime handshake was rejected with HTTP {status_code}"
        );
        let mut slot = ClientSlot {
            handle: self.client.clone(),
        };
        close_after_handshake_refusal(&mut slot, status_code)
            .await
            .map_err(RealtimeConnectionError::Contract)
    }

    // LiteLLM's generic catch: a redacted server_error event, then a redacted and truncated 1011 close whose only ignorable failure is a completed close
    async fn handle_failure(
        &self,
        error: &RealtimeConnectionError<Config::Error, Callback::Error, Relay::Error>,
    ) -> Result<(), RealtimeConnectionError<Config::Error, Callback::Error, Relay::Error>> {
        tracing::error!("Error connecting to backend: {error}");
        let message = self.redactor.redact_string(&error.to_string());
        let event = realtime_error_event(&message, "server_error")
            .map_err(RealtimeConnectionError::Contract)?;
        if let Err(send_error) = self.client.send_text(&event).await {
            tracing::debug!(
                "Could not send realtime error event to client; closing anyway: {send_error}"
            );
        }
        let detail = self
            .redactor
            .redact_string(&format!("Internal server error: {error}"));
        let reason = websocket_close_reason(&detail, "Internal server error");
        match self.client.close(1011, Some(&reason)).await {
            Ok(()) => Ok(()),
            Err(close_error) if is_ignorable_close(&close_error) => Ok(()),
            Err(close_error @ ClientCloseError::Runtime { .. }) => {
                Err(RealtimeConnectionError::UnexpectedClose { close_error })
            }
            Err(close_error) => Err(RealtimeConnectionError::Close(close_error)),
        }
    }
}
