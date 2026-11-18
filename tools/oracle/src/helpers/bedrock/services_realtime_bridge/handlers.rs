use std::future::Future;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_bedrock::realtime_bridge::error::{ClientForwardError, ProviderForwardError};
use betterlite_bedrock::realtime_bridge::provider::{LoggedEventSink, ProviderForward};
use betterlite_bedrock::realtime_bridge::{client::ClientForward, end::ForwardEnd};
use betterlite_bedrock::{realtime_events::scope::EventScope, realtime_input::RealtimeInput};
use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_core::api::realtime_contracts::events::RealtimeResponseTransformInput;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use futures_util::future::{Either, select};
use tokio::{runtime::Builder, task::yield_now};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::transports::{ClientState, StreamState, TransportClient, TransportStream, states};
use super::{
    BridgeView, ClosedClient, Input, Kind, Run, ScopeView, empty_state, failure, session_input,
};
use crate::services_realtime_session::FixedIds;
use betterlite_oracle_helper_api::error::HelperError;

#[derive(Default)]
struct CollectedLog {
    events: Vec<Json>,
}

#[async_trait]
impl LoggedEventSink for CollectedLog {
    async fn record(&mut self, event: Json) {
        self.events.push(event);
    }
}

// Cancels once the scripted side has been pulled as many times as the run says
async fn cancel_at(counter: &AtomicUsize, threshold: usize, cancel: &CancellationToken) {
    while counter.load(Ordering::SeqCst) < threshold {
        yield_now().await;
    }
    cancel.cancel();
}

fn parse_sent(values: Vec<String>) -> Result<Vec<Json>, Box<HelperError>> {
    values
        .into_iter()
        .map(|text| serde_json::from_str(&text).map_err(failure))
        .collect()
}

fn scope_view(scope: &EventScope) -> ScopeView {
    ScopeView {
        pending_session_update: match &scope.pending_session_update {
            Json::Text(text) => Some(text.clone()),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
        },
        session_committed: matches!(scope.session_committed, Json::Bool(true)),
    }
}

type ClientRun = Result<ForwardEnd, ClientForwardError<io::Error>>;
type ProviderRun = Result<ForwardEnd, ProviderForwardError<io::Error>>;

enum PumpResult<'result> {
    Client(&'result ClientRun),
    Provider(&'result ProviderRun),
}

// The pump's outcome with nothing observed yet, for the direction that fills only its own side
const fn pump_outcome(pump: PumpResult<'_>) -> BridgeView {
    let (outcome, error) = match pump {
        PumpResult::Client(run) => match run {
            Ok(ForwardEnd::Ended) => ("ended", None),
            Ok(ForwardEnd::Cancelled) => ("cancelled", None),
            Err(ClientForwardError::Input(_)) => ("error", Some("input")),
            Err(ClientForwardError::Ack(_)) => ("error", Some("ack")),
            Err(ClientForwardError::Send(_)) => ("error", Some("stream")),
            Err(ClientForwardError::Encode { .. }) => ("error", Some("encode")),
        },
        PumpResult::Provider(run) => match run {
            Ok(ForwardEnd::Ended) => ("ended", None),
            Ok(ForwardEnd::Cancelled) => ("cancelled", None),
            Err(ProviderForwardError::Stream(_)) => ("error", Some("stream")),
            Err(ProviderForwardError::Utf8 { .. }) => ("error", Some("utf8")),
            Err(ProviderForwardError::Transform(_)) => ("error", Some("transform")),
        },
    };
    BridgeView {
        outcome,
        error,
        sent_to_bedrock: Vec::new(),
        sent_to_client: Vec::new(),
        closed_input: false,
        closed_client: None,
        scope: None,
        state: None,
        logged: Vec::new(),
    }
}

// Both directions share one scripted transport pair, so one harness builds either pump per run
struct Harness<'case> {
    run: &'case Run,
    shared: Mutex<RealtimeInput>,
    client: Arc<ClientState>,
    wire: Arc<StreamState>,
    stream: TransportStream,
    transport: TransportClient,
    token: CancellationToken,
    events: &'case FixedIds,
    redactor: &'case Redactor,
    model: &'case str,
}

impl Harness<'_> {
    // A run that cancels stops the pump once the scripted side has been pulled as many times as it says
    async fn driven<T, F>(&self, counter: &AtomicUsize, pump: F) -> T
    where
        F: Future<Output = T>,
    {
        if !self.run.cancel.0 {
            return pump.await;
        }
        let pulls = match self.run.kind {
            Kind::Client => self.run.frames.len(),
            Kind::Provider => self.run.chunks.len(),
        };
        let canceller = cancel_at(counter, pulls.saturating_add(1), &self.token);
        tokio::pin!(pump);
        tokio::pin!(canceller);
        match select(pump, canceller).await {
            Either::Left((result, _canceller)) => result,
            Either::Right(((), pinned)) => pinned.await,
        }
    }

    // Each direction runs its own pump over this run's scripted transports and observes its own side
    async fn observe(&self) -> Result<BridgeView, Box<HelperError>> {
        match self.run.kind {
            Kind::Client => {
                let mut scope = EventScope::default();
                let mut forward: ClientForward<'_, io::Error, io::Error> = ClientForward {
                    client: &self.transport,
                    stream: &self.stream,
                    shared: &self.shared,
                    scope: &mut scope,
                    model: self.model,
                    trace_id: self.run.trace_id.as_deref(),
                    ids: self.events,
                    redactor: self.redactor,
                    initial: self.run.initial.clone(),
                    cancel: &self.token,
                };
                let future = forward.forward_client_to_bedrock();
                let result = self.driven(&self.client.receives, future).await;
                self.client_bridge(&scope, &result)
            }
            Kind::Provider => {
                let (mut state, mut log) = (empty_state(), CollectedLog::default());
                let mut pump: ProviderForward<'_, io::Error, io::Error> = ProviderForward {
                    stream: &self.stream,
                    client: &self.transport,
                    shared: &self.shared,
                    state: &mut state,
                    logged: &mut log,
                    logged_types: &LoggedEventTypes::Default,
                    ids: self.events,
                    redactor: self.redactor,
                    cancel: &self.token,
                };
                let future = pump.forward_bedrock_to_client();
                let result = self.driven(&self.wire.receives, future).await;
                self.provider_bridge(state, log, &result)
            }
        }
    }

    fn client_bridge(
        &self,
        scope: &EventScope,
        result: &ClientRun,
    ) -> Result<BridgeView, Box<HelperError>> {
        let mut view = pump_outcome(PumpResult::Client(result));
        view.sent_to_bedrock = self.wire.sent();
        view.sent_to_client = parse_sent(self.client.sent())?;
        view.closed_input = self.wire.closed.load(Ordering::SeqCst);
        view.scope = Some(scope_view(scope));
        Ok(view)
    }

    fn provider_bridge(
        &self,
        state: RealtimeResponseTransformInput,
        log: CollectedLog,
        result: &ProviderRun,
    ) -> Result<BridgeView, Box<HelperError>> {
        let mut view = pump_outcome(PumpResult::Provider(result));
        view.sent_to_client = parse_sent(self.client.sent())?;
        // The provider pump closes the client at most once, and only on a normal provider end
        view.closed_client = self
            .client
            .closes()
            .first()
            .map(|(code, reason)| ClosedClient {
                code: *code,
                reason: reason.clone(),
            });
        view.state = Some(state);
        view.logged = log.events;
        Ok(view)
    }
}

async fn view(
    run: &Run,
    name_ids: &[Uuid],
    events: &FixedIds,
    redactor: &Redactor,
    model: &str,
) -> Result<BridgeView, Box<HelperError>> {
    let (client, wire) = states(run);
    let stream = TransportStream::new(Arc::clone(&wire));
    let transport = TransportClient {
        state: Arc::clone(&client),
    };
    let harness = Harness {
        run,
        shared: Mutex::new(session_input(name_ids)),
        client,
        wire,
        stream,
        transport,
        token: CancellationToken::new(),
        events,
        redactor,
        model,
    };
    harness.observe().await
}

pub(super) fn runs(
    input: &Input,
    names: &[Uuid],
    events: &FixedIds,
    redactor: &Redactor,
) -> Result<serde_json::Value, Box<HelperError>> {
    let executor = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(failure)?;
    let views = executor.block_on(async {
        let mut views = Vec::with_capacity(input.runs.len());
        for (index, run) in input.runs.iter().enumerate() {
            let start = index.saturating_mul(3);
            let name_ids = names
                .get(start..start.saturating_add(3))
                .ok_or_else(|| failure(format!("run {index} needs three name ids")))?;
            views.push(view(run, name_ids, events, redactor, &input.model).await?);
        }
        Ok::<Vec<BridgeView>, Box<HelperError>>(views)
    })?;
    serde_json::to_value(super::Runs { runs: views }).map_err(failure)
}
