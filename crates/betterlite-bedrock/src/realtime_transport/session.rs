use std::error::Error as StdError;
use std::fmt::Display;
use std::mem;
use std::sync::{Mutex, PoisonError};

use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_core::api::realtime_contracts::events::RealtimeResponseTransformInput;
use betterlite_core::api::realtime_session_state::logging::{
    RealtimeLogging, RealtimeSuccessQueue, SUCCESS_LOGGED_KEY,
};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;

use super::bridge::Bridge;
use super::error::{NO_CREDENTIALS_MESSAGE, ProviderClass, RealtimeTransportError};
use super::prepare::{Preparation, RealtimeCredentials, prepare};
use super::runtime::{RealtimeClient, RealtimeRuntime, RuntimeConfig};
use crate::agentcore_stream::IdSource;
use crate::aws_settings::AuthParams;
use crate::realtime_bridge::end::ForwardEnd;
use crate::realtime_bridge::error::{ClientForwardError, ProviderForwardError};
use crate::realtime_completion::records::CompletionEvent;
use crate::realtime_completion::usage::leftover_usage_done_events;
use crate::realtime_events::ack::AckError;
use crate::realtime_events::client::RealtimeClientWebSocket;
use crate::realtime_events::close::close_bedrock_client;
use crate::realtime_events::failure::{ProviderFailure, raise_provider_failure};
use crate::realtime_events::logging::should_log_event;
use crate::realtime_events::scope::EventScope;
use crate::realtime_events::stream::StreamError;
use crate::realtime_input::RealtimeInput;
use crate::realtime_session::RealtimeSession;
use crate::realtime_session::records::SessionEventType;
use crate::signing::target::{RegionInputs, auth_settings};

pub struct RealtimeCall<'call, C>
where
    C: StdError + Send + Sync + 'static,
{
    pub model: &'call str,
    pub client: &'call dyn RealtimeClientWebSocket<Error = C>,
    pub logging: &'call dyn RealtimeLogging,
    pub logging_worker: Option<&'call dyn RealtimeSuccessQueue>,
    pub api_base: Option<&'call str>,
    pub aws_bedrock_runtime_endpoint: Option<&'call str>,
    pub aws_region_name: Option<&'call str>,
    pub auth: AuthParams,
    pub region_inputs: RegionInputs<'call>,
    pub scope: &'call mut EventScope,
    pub ids: &'call dyn IdSource,
    pub redactor: &'call Redactor,
    pub logged_types: &'call LoggedEventTypes,
    pub trace_id: &'call str,
}

pub struct RealtimeTransport<'deps, R, K> {
    pub runtime: &'deps R,
    pub credentials: &'deps K,
}

impl<'deps, R, K> RealtimeTransport<'deps, R, K>
where
    R: RealtimeRuntime,
    K: RealtimeCredentials,
{
    #[must_use]
    pub const fn new(runtime: &'deps R, credentials: &'deps K) -> Self {
        Self {
            runtime,
            credentials,
        }
    }

    // async_realtime: prepare, find credentials, build the SDK client, run the session, close the client
    pub async fn realtime<C>(
        &self,
        mut call: RealtimeCall<'_, C>,
    ) -> Result<(), RealtimeTransportError<R::Error, C>>
    where
        C: StdError + Send + Sync + 'static,
    {
        // A committed scope answers before any credential or connection work
        let pending = call.scope.pending_session_update()?;
        let target = prepare(Preparation {
            model: call.model,
            api_base: call.api_base,
            aws_bedrock_runtime_endpoint: call.aws_bedrock_runtime_endpoint,
            aws_region_name: call.aws_region_name,
            region_inputs: call.region_inputs,
        })?;
        tracing::debug!(
            "Bedrock Realtime: Connecting to {} with model {}",
            target.endpoint_uri,
            call.model
        );
        let settings = auth_settings(mem::take(&mut call.auth), &target.region);
        let Some(credentials) = self.credentials.resolve(&settings, &target.region).await? else {
            return Err(RealtimeTransportError::NoCredentials {
                message: NO_CREDENTIALS_MESSAGE.to_owned(),
                status_code: 401,
            });
        };
        let config = RuntimeConfig {
            endpoint_uri: target.endpoint_uri,
            region: target.region,
            credentials,
        };
        let client = self.runtime.connect(config).await?;
        let outcome = run_session(&mut call, &*client, pending.as_deref()).await;
        close_bedrock_client(Some(client.closable())).await;
        outcome
    }
}

async fn run_session<E, C>(
    call: &mut RealtimeCall<'_, C>,
    client: &dyn RealtimeClient<Error = E>,
    pending: Option<&str>,
) -> Result<(), RealtimeTransportError<E, C>>
where
    E: StdError + Send + Sync + 'static + ProviderClass,
    C: StdError + Send + Sync + 'static,
{
    let stream = client.invoke(call.model).await?;
    tracing::debug!("Bedrock Realtime: Bidirectional stream established");
    // An empty pending string still suppresses the event: only None means the client got no session yet
    if pending.is_none() {
        let event = RealtimeSession::session_event(
            SessionEventType::Created,
            Some(call.model),
            call.trace_id,
            None,
            call.ids,
        );
        let text = serde_json::to_string(&event)
            .map_err(|source| RealtimeTransportError::SessionCreated { source })?;
        call.client.send_text(text).await?;
        tracing::debug!("Bedrock Realtime: sent session.created to client on connect");
    }
    let shared = Mutex::new(RealtimeInput {
        session: RealtimeSession::new(call.ids),
        open_audio: None,
    });
    let mut state = initial_state();
    let runs = Bridge {
        client: call.client,
        stream: &*stream,
        shared: &shared,
        scope: &mut *call.scope,
        state: &mut state,
        model: call.model,
        trace_id: call.trace_id,
        ids: call.ids,
        redactor: call.redactor,
        logged_types: call.logged_types,
        initial: pending.map(str::to_owned),
    }
    .run()
    .await;
    let mut input = shared.into_inner().unwrap_or_else(PoisonError::into_inner);
    let leftovers = leftover_usage_done_events(&mut input.session, call.ids)?;
    let mut logged_events = runs.logged_events;
    for event in &leftovers {
        let record = logging_record(event)?;
        if should_log_event(call.logged_types, &record) {
            logged_events.push(record);
        }
    }
    // Success handlers run before a provider failure is raised, so a failed session still records served usage
    if !logged_events.is_empty() {
        let work = call.logging.dispatch_success_handlers(&logged_events);
        let accepted = call.logging_worker.is_some_and(|queue| queue.enqueue(work));
        if accepted {
            call.logging
                .record_model_call_detail(SUCCESS_LOGGED_KEY, Json::Bool(true));
        }
    }
    let Some(failure) = provider_failure(runs.client, runs.provider) else {
        return Ok(());
    };
    // A provider failure after the client left is the client's business, not the session's
    if runs.client_disconnected {
        tracing::debug!("Bedrock Realtime: stream failed after the client disconnected: {failure}");
        return Ok(());
    }
    Err(raise_provider_failure(&mut *call.scope, failure, call.redactor).into())
}

// _run_session opens every attempt with no transformation state, whatever the previous attempt left
#[must_use]
pub(crate) fn initial_state() -> RealtimeResponseTransformInput {
    RealtimeResponseTransformInput {
        session_configuration_request: None,
        current_output_item_id: None,
        current_response_id: None,
        current_delta_chunks: None,
        current_item_chunks: None,
        current_conversation_id: None,
        current_delta_type: None,
    }
}

fn logging_record<E, C>(
    event: &CompletionEvent,
) -> Result<IndexMap<String, Json>, RealtimeTransportError<E, C>>
where
    E: StdError + 'static,
    C: StdError + 'static,
{
    let value =
        serde_json::to_value(event).map_err(|source| RealtimeTransportError::Logging { source })?;
    serde_json::from_value(value).map_err(|source| RealtimeTransportError::Logging { source })
}

fn provider_failure<E>(
    client: Result<ForwardEnd, ClientForwardError<E>>,
    provider: Result<ForwardEnd, ProviderForwardError<E>>,
) -> Option<ProviderFailure<E>>
where
    E: StdError + 'static + ProviderClass,
{
    match client {
        Err(error) => Some(client_failure(error)),
        Ok(_) => match provider {
            Err(error) => Some(provider_forward_failure(error)),
            Ok(_) => None,
        },
    }
}

fn stream_failure<E>(error: StreamError<E>) -> ProviderFailure<E>
where
    E: StdError + 'static + ProviderClass,
{
    let StreamError { message, source } = error;
    let class_name = source
        .as_ref()
        .and_then(|error| error.provider_class_name().map(str::to_owned))
        .unwrap_or_else(|| "RuntimeError".to_owned());
    let message = source
        .as_ref()
        .and_then(|error| error.provider_message().map(str::to_owned))
        .unwrap_or(message);
    ProviderFailure {
        class_name,
        message,
        source,
    }
}

fn named_failure<E>(class_name: &str, error: &dyn Display) -> ProviderFailure<E> {
    ProviderFailure {
        class_name: class_name.to_owned(),
        message: error.to_string(),
        source: None,
    }
}

fn client_failure<E>(error: ClientForwardError<E>) -> ProviderFailure<E>
where
    E: StdError + 'static + ProviderClass,
{
    match error {
        ClientForwardError::Send(error) => stream_failure(error),
        ClientForwardError::Ack(AckError::Stream(error)) => stream_failure(error),
        ClientForwardError::Input(error) => named_failure("InputError", &error),
        ClientForwardError::Ack(error) => named_failure("AckError", &error),
        ClientForwardError::Encode { source } => named_failure("EncodeError", &source),
    }
}

fn provider_forward_failure<E>(error: ProviderForwardError<E>) -> ProviderFailure<E>
where
    E: StdError + 'static + ProviderClass,
{
    match error {
        ProviderForwardError::Stream(error) => stream_failure(error),
        ProviderForwardError::Utf8 { source } => named_failure("UnicodeDecodeError", &source),
        ProviderForwardError::Transform(error) => named_failure("EventError", &error),
    }
}
