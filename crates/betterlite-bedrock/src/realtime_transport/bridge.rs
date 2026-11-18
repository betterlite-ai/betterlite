use std::error::Error as StdError;
use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_core::api::realtime_contracts::events::RealtimeResponseTransformInput;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use futures_util::future::{Either, select};
use indexmap::IndexMap;
use tokio_util::sync::CancellationToken;

use crate::agentcore_stream::IdSource;
use crate::realtime_bridge::client::ClientForward;
use crate::realtime_bridge::end::ForwardEnd;
use crate::realtime_bridge::error::{ClientForwardError, ProviderForwardError};
use crate::realtime_bridge::provider::{LoggedEventSink, ProviderForward};
use crate::realtime_events::client::RealtimeClientWebSocket;
use crate::realtime_events::scope::EventScope;
use crate::realtime_events::stream::BedrockBidirectionalStream;
use crate::realtime_input::RealtimeInput;

pub struct Bridge<'run, C, S>
where
    C: StdError + Send + Sync + 'static,
    S: StdError + Send + Sync + 'static,
{
    pub client: &'run dyn RealtimeClientWebSocket<Error = C>,
    pub stream: &'run dyn BedrockBidirectionalStream<Error = S>,
    pub shared: &'run Mutex<RealtimeInput>,
    pub scope: &'run mut EventScope,
    pub state: &'run mut RealtimeResponseTransformInput,
    pub model: &'run str,
    pub trace_id: &'run str,
    pub ids: &'run dyn IdSource,
    pub redactor: &'run Redactor,
    pub logged_types: &'run LoggedEventTypes,
    pub initial: Option<String>,
}

pub struct BridgeRuns<S>
where
    S: StdError + 'static,
{
    pub logged_events: Vec<IndexMap<String, Json>>,
    pub client: Result<ForwardEnd, ClientForwardError<S>>,
    pub provider: Result<ForwardEnd, ProviderForwardError<S>>,
    pub client_disconnected: bool,
}

impl<C, S> Bridge<'_, C, S>
where
    C: StdError + Send + Sync + 'static,
    S: StdError + Send + Sync + 'static,
{
    // Both directions run in this future; whichever finishes first cancels the other and then both are joined
    pub async fn run(self) -> BridgeRuns<S> {
        let Bridge {
            client,
            stream,
            shared,
            scope,
            state,
            model,
            trace_id,
            ids,
            redactor,
            logged_types,
            initial,
        } = self;
        let cancel = CancellationToken::new();
        let mut logged = RecordedEvents::default();
        let mut client_pump = ClientForward {
            client,
            stream,
            shared,
            scope,
            model,
            trace_id: Some(trace_id),
            ids,
            redactor,
            initial,
            cancel: &cancel,
        };
        let mut provider_pump = ProviderForward {
            stream,
            client,
            shared,
            state,
            logged: &mut logged,
            logged_types,
            ids,
            redactor,
            cancel: &cancel,
        };
        let mut client_future = Box::pin(client_pump.forward_client_to_bedrock());
        let mut provider_future = Box::pin(provider_pump.forward_bedrock_to_client());
        let mut client_outcome = None;
        let mut provider_outcome = None;
        match select(client_future.as_mut(), provider_future.as_mut()).await {
            Either::Left((outcome, _pending)) => client_outcome = Some(outcome),
            Either::Right((outcome, _pending)) => provider_outcome = Some(outcome),
        }
        // The source's wait also collects a task that finished in the same loop iteration; one poll reads it before the cancel
        if client_outcome.is_none() {
            if let Either::Left((outcome, _ready)) =
                select(client_future.as_mut(), std::future::ready(())).await
            {
                client_outcome = Some(outcome);
            }
        }
        if provider_outcome.is_none() {
            if let Either::Left((outcome, _ready)) =
                select(provider_future.as_mut(), std::future::ready(())).await
            {
                provider_outcome = Some(outcome);
            }
        }
        let client_disconnected = matches!(client_outcome, Some(Ok(ForwardEnd::Ended)));
        cancel.cancel();
        if client_outcome.is_none() {
            client_outcome = Some(client_future.await);
        }
        if provider_outcome.is_none() {
            provider_outcome = Some(provider_future.await);
        }
        drop(client_future);
        drop(provider_future);
        drop(client_pump);
        drop(provider_pump);
        BridgeRuns {
            logged_events: logged.events,
            client: client_outcome.unwrap_or(Ok(ForwardEnd::Cancelled)),
            provider: provider_outcome.unwrap_or(Ok(ForwardEnd::Cancelled)),
            client_disconnected,
        }
    }
}

#[derive(Default)]
struct RecordedEvents {
    events: Vec<IndexMap<String, Json>>,
}

#[async_trait]
impl LoggedEventSink for RecordedEvents {
    async fn record(&mut self, event: Json) {
        if let Json::Object(fields) = event {
            self.events.push(fields);
        }
    }
}
