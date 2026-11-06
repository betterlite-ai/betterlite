use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_mcp::client::cleanup::failure::ErrorContext;
use betterlite_mcp::client::sessions::client::SessionClient;
use betterlite_mcp::client::sessions::error::{McpFailure, SessionError};
use betterlite_mcp::client::sessions::execute::{
    SESSION_CLEANUP_TIMEOUT, execute_session_operation,
};
use betterlite_mcp::client::sessions::run::{HTTP_CLOSE_TIMEOUT, run_with_session};
use betterlite_mcp::client::sessions::stream::{StreamFailure, StreamFailureKind, StreamMessage};
use betterlite_mcp::client::sessions::transport::{TransportContexts, TransportType};
use serde_json::{Value, json};
use tokio::runtime::Builder;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::Subscriber;
use tracing::subscriber::set_default;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::{Context, SubscriberExt as _};

use crate::helpers::{HelperError, HelperResult};

use super::super::fixtures::{FakeSession, Fixture, Script, TestError};
use super::super::input::{ExecuteInput, Mode, Operation, Scenario, SettingsInput, StreamKind};
use super::{invalid, make_client};

#[derive(Clone, Default)]
struct Counts {
    warn: Arc<AtomicUsize>,
    debug: Arc<AtomicUsize>,
}

impl<Current> Layer<Current> for Counts
where
    Current: Subscriber,
{
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, Current>) {
        match *event.metadata().level() {
            tracing::Level::WARN => {
                self.warn.fetch_add(1, Ordering::Relaxed);
            }
            tracing::Level::DEBUG => {
                self.debug.fetch_add(1, Ordering::Relaxed);
            }
            tracing::Level::ERROR | tracing::Level::INFO | tracing::Level::TRACE => {}
        }
    }
}

fn stream_failure(kind: StreamKind) -> StreamFailure<TestError> {
    let source = Arc::new(TestError::Boom("stream"));
    match kind {
        StreamKind::Value => StreamFailure::ValueError(source),
        StreamKind::Http => StreamFailure::Http(source),
        StreamKind::Io => StreamFailure::Io(source),
    }
}

const fn stream_name(failure: &StreamFailure<TestError>) -> &'static str {
    match failure.kind() {
        StreamFailureKind::ValueError => "value",
        StreamFailureKind::Http => "http",
        StreamFailureKind::Io => "io",
    }
}

fn outcome_kind(error: &SessionError<TestError>) -> String {
    match error {
        SessionError::Protocol(failure) => format!("protocol:{}", failure.code),
        SessionError::Runtime(source) => match source.as_ref() {
            TestError::Boom(label) => format!("runtime:{label}"),
            TestError::Protocol(_) => "runtime:mcp_error".to_owned(),
        },
        SessionError::Stream(failure) => format!("stream:{}", stream_name(failure.as_ref())),
        SessionError::Cancelled => "cancelled".to_owned(),
        SessionError::CleanupTimedOut => "timeout".to_owned(),
        SessionError::ReadTimeout(_) => "read_timeout".to_owned(),
        SessionError::CleanupTask(_) => "task".to_owned(),
        SessionError::Configuration(_) => "configuration".to_owned(),
    }
}

fn protocol_error(code: i32, context: Option<ErrorContext>) -> SessionError<TestError> {
    SessionError::Protocol(McpFailure {
        code,
        message: "upstream error".to_owned(),
        context,
    })
}

async fn lifecycle(
    client: &mut SessionClient<Fixture>,
    contexts: TransportContexts<Fixture>,
    script: Arc<Script>,
    scenario: &Scenario,
) -> Result<&'static str, SessionError<TestError>> {
    let operation =
        async |session: &mut FakeSession| -> Result<&'static str, SessionError<TestError>> {
            script.record("operation");
            if let Some(kind) = scenario.stream {
                let _failed = session
                    .handler
                    .handle(StreamMessage::Failure(stream_failure(kind)));
            }
            match scenario.operation {
                Operation::Ok => Ok("done"),
                Operation::Protocol => Err(protocol_error(-32_000, None)),
                Operation::Application => Err(SessionError::Runtime(Arc::new(TestError::Boom(
                    "application",
                )))),
                Operation::Cancel => Err(SessionError::Cancelled),
                Operation::ReadTimeout => Err(protocol_error(-32_001, Some(ErrorContext::Timeout))),
                Operation::RelayedTimeout => Err(protocol_error(-32_001, None)),
            }
        };
    match scenario.mode {
        Mode::Execute => execute_session_operation(client, contexts, operation).await,
        Mode::Run => run_with_session(client, operation, scenario.quiet).await,
    }
}

struct Run {
    outcome: Result<&'static str, SessionError<TestError>>,
    calls: Vec<String>,
    warnings: usize,
    debugs: usize,
    instructions: Option<String>,
}

fn scenario_run(settings: &SettingsInput, scenario: &Scenario) -> Result<Run, Box<HelperError>> {
    let mut relaxed = settings.clone();
    let active = if scenario.mode == Mode::Run && !scenario.http {
        relaxed.transport_type = TransportType::Sse;
        &relaxed
    } else {
        settings
    };
    let fixture = Fixture::new(scenario.plan.clone());
    let contexts = fixture.contexts(scenario.http);
    let script = Arc::clone(&fixture.script);
    let mut client = make_client(
        active,
        fixture,
        TaskTracker::new(),
        CancellationToken::new(),
    )?;
    client.last_initialize_instructions = Some("stale".to_owned());
    let runtime = Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .map_err(|error| invalid(error.to_string()))?;
    let counts = Counts::default();
    let outcome = {
        let _guard = set_default(tracing_subscriber::registry().with(counts.clone()));
        runtime.block_on(lifecycle(
            &mut client,
            contexts,
            Arc::clone(&script),
            scenario,
        ))
    };
    Ok(Run {
        outcome,
        calls: script.calls(),
        warnings: counts.warn.load(Ordering::Relaxed),
        debugs: counts.debug.load(Ordering::Relaxed),
        instructions: client.last_initialize_instructions,
    })
}

fn scenario_json(scenario: &Scenario, run: &Run) -> Value {
    let (kind, value) = match &run.outcome {
        Ok(value) => ("ok".to_owned(), Some(*value)),
        Err(error) => (outcome_kind(error), None),
    };
    let bound = u64::try_from(SESSION_CLEANUP_TIMEOUT.as_millis()).unwrap_or(u64::MAX);
    let session_bound = run
        .calls
        .iter()
        .any(|call| call == "session.exit")
        .then_some(bound);
    let transport_bound = run
        .calls
        .iter()
        .any(|call| call == "transport.exit")
        .then_some(bound);
    let http_bound = (scenario.mode == Mode::Run && scenario.http)
        .then_some(u64::try_from(HTTP_CLOSE_TIMEOUT.as_millis()).unwrap_or(u64::MAX));
    json!({
        "calls": run.calls,
        "warnings": run.warnings,
        "debugs": run.debugs,
        "bounds_ms": {
            "session": session_bound,
            "transport": transport_bound,
            "http": http_bound,
        },
        "instructions": run.instructions,
        "outcome": kind,
        "value": value,
    })
}

pub(crate) fn execute(input: &ExecuteInput) -> HelperResult {
    let mut results = Vec::new();
    for scenario in &input.scenarios {
        let run = scenario_run(&input.settings, scenario)?;
        results.push(scenario_json(scenario, &run));
    }
    Ok(json!({"results": results}))
}
