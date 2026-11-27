use super::answer::{AdapterUnavailable, answered};
use super::convert::{ConvertError, NativeErrorPayload, SessionWire, parse};
use super::error::{SessionError, payload_error};
use super::logging::{SilentSink, logging_state};
use super::provider::UnavailableAws;
use super::stream::StreamKind;
use crate::completion::ports::SystemIds;
use betterlite_config::env_integer::timeout::RequestTimeout;
use betterlite_config::loader::startup::process_environment;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::Section as _;
use betterlite_config::sections::studio_identity::Raw as StudioRaw;
use betterlite_config::sections::vertex_credential_sources::Raw as VertexRaw;
use betterlite_config::sections::vertex_schema_shaping::Raw as SchemaRaw;
use betterlite_config::tls::user_agent;
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::vector_store_http_write::service::LogContext;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::BundledCatalog;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_sdk::call_wrapper::ports::IdSource;
use betterlite_vertex::credential_sources::sdk::SystemClock;
use betterlite_vertex::genai::session::call::GenaiCall;
use betterlite_vertex::genai::session::clients::NativeClients;
use betterlite_vertex::genai::session::{Entry, GenaiSession, SessionParts, logging_env};
use pyo3::PyErr;
use secrecy::SecretString;
use std::sync::Arc;

// What a finished call hands back: the response text Python loads, or the stream it iterates.
pub(crate) enum Outcome {
    Text(String),
    Stream(StreamKind),
}

// The session litellm.google_genai's four entry points hold, and the source it mints call ids from.
pub(crate) struct GenaiCore {
    session: GenaiSession,
    ids: Arc<dyn IdSource>,
}

impl GenaiCore {
    pub(crate) fn build(text: &str) -> Result<Self, SessionError> {
        let wire: SessionWire = parse(text)?;
        let environment = wire.environment.unwrap_or_else(process_environment);
        let mut studio = StudioRaw::default()
            .convert(&environment)
            .map_err(|source| SessionError::Settings { source })?;
        // litellm.api_key: the last studio key LiteLLM reads, after the GOOGLE_API_KEY and GEMINI_API_KEY environment keys
        studio.generic_api_key = wire
            .api_key
            .map(SecretString::from)
            .or(studio.generic_api_key);
        let vertex = VertexRaw::default()
            .convert(&environment)
            .map_err(|source| SessionError::Settings { source })?;
        let schema = SchemaRaw::default()
            .convert(&environment)
            .map_err(|source| SessionError::Settings { source })?;
        let request_timeout = RequestTimeout::resolve(&environment)
            .map_err(|source| SessionError::Timeout { source })?;
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
            .map_err(|source| SessionError::Redactor { source })?;
        let policy = Arc::new(
            ProxyPolicy::resolve(&ProxyTrust::default(), &environment)
                .map_err(|source| SessionError::Proxy { source })?,
        );
        let default =
            ProxiedClient::outbound(&policy).map_err(|source| SessionError::Client { source })?;
        let clients = NativeClients::new(default, user_agent(&environment));
        let session = GenaiSession::new(SessionParts {
            environment,
            studio,
            vertex,
            schema,
            request_timeout,
            catalog: BundledCatalog::default().get(),
            clients,
            clock: Arc::new(SystemClock),
            redactor,
            aws: Arc::new(UnavailableAws),
        });
        Ok(Self {
            session,
            ids: Arc::new(SystemIds),
        })
    }

    // one call: the timeout, the logging state, the answer and the exception a failure becomes
    pub(crate) async fn call(&self, call: GenaiCall, entry: Entry) -> Result<Outcome, PyErr> {
        let timeout = call
            .timeout()
            .map_err(|source| ConvertError::Timeout { source })?;
        let request = call.request(timeout);
        let mut state = logging_state(&call, entry, &self.session.redactor, self.ids.as_ref())?;
        let sink = SilentSink;
        let env = logging_env(&self.session.redactor, &sink);
        let mut log = LogContext {
            state: &mut state,
            env: &env,
        };
        let answer = self
            .session
            .run(&request, &AdapterUnavailable, &mut log, entry)
            .await
            .map_err(|failure| payload_error(&NativeErrorPayload::from_class(&failure.class)))?;
        answered(answer)
    }
}
