use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_bedrock::realtime_bridge::client::ClientForward;
use betterlite_bedrock::realtime_bridge::end::ForwardEnd;
use betterlite_bedrock::realtime_bridge::error::{ClientForwardError, ProviderForwardError};
use betterlite_bedrock::realtime_bridge::provider::{LoggedEventSink, ProviderForward};
use betterlite_bedrock::realtime_events::scope::EventScope;
use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_core::api::realtime_contracts::events::RealtimeResponseTransformInput;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use tokio::runtime::{Builder, Runtime};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::transports::{TransportClient, TransportStream, states};
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

fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(failure)
}

fn threshold(run: &Run) -> usize {
    match run.kind {
        Kind::Client => run.frames.len().saturating_add(1),
        Kind::Provider => run.chunks.len().saturating_add(1),
    }
}

// Cancels once the scripted side has been pulled as many times as the run says
async fn cancel_at(counter: &AtomicUsize, threshold: usize, cancel: &CancellationToken) {
    while counter.load(Ordering::SeqCst) < threshold {
        tokio::task::yield_now().await;
    }
    cancel.cancel();
}

fn parse_sent(values: Vec<String>) -> Result<Vec<Json>, Box<HelperError>> {
    values
        .iter()
        .map(|text| serde_json::from_str(text).map_err(failure))
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

enum PumpResult<'a> {
    Client(&'a Result<ForwardEnd, ClientForwardError<io::Error>>),
    Provider(&'a Result<ForwardEnd, ProviderForwardError<io::Error>>),
}

fn pump_outcome(result: PumpResult<'_>) -> (&'static str, Option<&'static str>) {
    match result {
        PumpResult::Client(result) => match result {
            Ok(ForwardEnd::Ended) => ("ended", None),
            Ok(ForwardEnd::Cancelled) => ("cancelled", None),
            Err(ClientForwardError::Input(_)) => ("error", Some("input")),
            Err(ClientForwardError::Ack(_)) => ("error", Some("ack")),
            Err(ClientForwardError::Send(_)) => ("error", Some("stream")),
            Err(ClientForwardError::Encode { .. }) => ("error", Some("encode")),
        },
        PumpResult::Provider(result) => match result {
            Ok(ForwardEnd::Ended) => ("ended", None),
            Ok(ForwardEnd::Cancelled) => ("cancelled", None),
            Err(ProviderForwardError::Stream(_)) => ("error", Some("stream")),
            Err(ProviderForwardError::Utf8 { .. }) => ("error", Some("utf8")),
            Err(ProviderForwardError::Transform(_)) => ("error", Some("transform")),
        },
    }
}

async fn view(
    run: &Run,
    name_ids: &[Uuid],
    events: &FixedIds,
    redactor: &Redactor,
    model: &str,
) -> Result<BridgeView, Box<HelperError>> {
    let shared = Mutex::new(session_input(name_ids));
    let (client, wire) = states(run);
    let stream = TransportStream::new(Arc::clone(&wire));
    let transport = TransportClient {
        state: Arc::clone(&client),
    };
    let token = CancellationToken::new();
    match run.kind {
        Kind::Client => {
            let mut scope = EventScope::default();
            let mut forward: ClientForward<'_, io::Error, io::Error> = ClientForward {
                client: &transport,
                stream: &stream,
                shared: &shared,
                scope: &mut scope,
                model,
                trace_id: run.trace_id.as_deref(),
                ids: events,
                redactor,
                initial: run.initial.clone(),
                cancel: &token,
            };
            let result = if run.cancel {
                let canceller = cancel_at(&client.receives, threshold(run), &token);
                let pump = forward.forward_client_to_bedrock();
                tokio::pin!(pump);
                tokio::pin!(canceller);
                tokio::select! {
                    result = &mut pump => result,
                    () = &mut canceller => pump.await,
                }
            } else {
                forward.forward_client_to_bedrock().await
            };
            let (outcome, error) = pump_outcome(PumpResult::Client(&result));
            Ok(BridgeView {
                outcome,
                error,
                sent_to_bedrock: wire.sent(),
                sent_to_client: parse_sent(client.sent())?,
                closed_input: wire.closed.load(Ordering::SeqCst),
                closed_client: None,
                scope: Some(scope_view(&scope)),
                state: None,
                logged: Vec::new(),
            })
        }
        Kind::Provider => {
            let mut state: RealtimeResponseTransformInput = empty_state();
            let mut log = CollectedLog::default();
            let types = LoggedEventTypes::Default;
            let mut forward: ProviderForward<'_, io::Error, io::Error> = ProviderForward {
                stream: &stream,
                client: &transport,
                shared: &shared,
                state: &mut state,
                logged: &mut log,
                logged_types: &types,
                ids: events,
                redactor,
                cancel: &token,
            };
            let result = if run.cancel {
                let canceller = cancel_at(&wire.receives, threshold(run), &token);
                let pump = forward.forward_bedrock_to_client();
                tokio::pin!(pump);
                tokio::pin!(canceller);
                tokio::select! {
                    result = &mut pump => result,
                    () = &mut canceller => pump.await,
                }
            } else {
                forward.forward_bedrock_to_client().await
            };
            let (outcome, error) = pump_outcome(PumpResult::Provider(&result));
            let closed = client.closes();
            Ok(BridgeView {
                outcome,
                error,
                sent_to_bedrock: Vec::new(),
                sent_to_client: parse_sent(client.sent())?,
                closed_input: false,
                closed_client: closed.first().map(|(code, reason)| ClosedClient {
                    code: *code,
                    reason: reason.clone(),
                }),
                scope: None,
                state: Some(state),
                logged: log.events,
            })
        }
    }
}

pub(super) fn runs(
    input: &Input,
    names: &[Uuid],
    events: &FixedIds,
    redactor: &Redactor,
) -> Result<serde_json::Value, Box<HelperError>> {
    let executor = runtime()?;
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
