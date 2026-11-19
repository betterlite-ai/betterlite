use std::error::Error as StdError;
use std::future::{Future, ready};
use std::pin::Pin;
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

impl<'run, C, S> Bridge<'run, C, S>
where
    C: StdError + Send + Sync + 'static,
    S: StdError + Send + Sync + 'static,
{
    // Both directions run in this future; whichever finishes first cancels the other and then both are joined
    #[must_use]
    pub async fn run(self) -> BridgeRuns<S> {
        let cancel = CancellationToken::new();
        let mut logged = RecordedEvents::default();
        let (mut client_pump, mut provider_pump) = self.pumps(&cancel, &mut logged);
        let mut client_future = Box::pin(client_pump.forward_client_to_bedrock());
        let mut provider_future = Box::pin(provider_pump.forward_bedrock_to_client());
        let mut client_outcome = None;
        let mut provider_outcome = None;
        settle(
            Pin::as_mut(&mut client_future),
            Pin::as_mut(&mut provider_future),
            &mut client_outcome,
            &mut provider_outcome,
        )
        .await;
        let client_disconnected = matches!(client_outcome, Some(Ok(ForwardEnd::Ended)));
        cancel.cancel();
        let client = finish(client_future, client_outcome).await;
        let provider = finish(provider_future, provider_outcome).await;
        BridgeRuns {
            logged_events: logged.events,
            client,
            provider,
            client_disconnected,
        }
    }

    fn pumps<'cancel>(
        self,
        cancel: &'cancel CancellationToken,
        logged: &'cancel mut RecordedEvents,
    ) -> (ClientForward<'cancel, C, S>, ProviderForward<'cancel, C, S>)
    where
        'run: 'cancel,
    {
        let client = ClientForward {
            client: self.client,
            stream: self.stream,
            shared: self.shared,
            scope: self.scope,
            model: self.model,
            trace_id: Some(self.trace_id),
            ids: self.ids,
            redactor: self.redactor,
            initial: self.initial,
            cancel,
        };
        let provider = ProviderForward {
            stream: self.stream,
            client: self.client,
            shared: self.shared,
            state: self.state,
            logged,
            logged_types: self.logged_types,
            ids: self.ids,
            redactor: self.redactor,
            cancel,
        };
        (client, provider)
    }
}

// The source's wait also collects a task that finished in the same loop iteration; one poll reads it before the cancel
async fn settle<A, B, T, U>(
    mut client: Pin<&mut A>,
    mut provider: Pin<&mut B>,
    client_outcome: &mut Option<T>,
    provider_outcome: &mut Option<U>,
) where
    A: Future<Output = T>,
    B: Future<Output = U>,
{
    match select(client.as_mut(), provider.as_mut()).await {
        Either::Left((outcome, _pending)) => *client_outcome = Some(outcome),
        Either::Right((outcome, _pending)) => *provider_outcome = Some(outcome),
    }
    if client_outcome.is_none()
        && let Either::Left((outcome, _ready)) = select(client.as_mut(), ready(())).await
    {
        *client_outcome = Some(outcome);
    }
    if provider_outcome.is_none()
        && let Either::Left((outcome, _ready)) = select(provider.as_mut(), ready(())).await
    {
        *provider_outcome = Some(outcome);
    }
}

async fn finish<F>(future: F, settled: Option<F::Output>) -> F::Output
where
    F: Future,
{
    match settled {
        Some(outcome) => outcome,
        None => future.await,
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
