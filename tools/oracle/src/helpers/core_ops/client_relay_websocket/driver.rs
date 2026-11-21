use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_core::api::realtime_backend_relay::relay_callbacks;
use betterlite_core::api::realtime_client_relay::RealtimeClientRelay;
use betterlite_core::api::realtime_contracts::close::{BackendClose, ClientLoopExit};
use betterlite_core::api::realtime_contracts::transport::WebSocketScope;
use betterlite_core::api::realtime_guardrails::callback::RealtimeGuardrail;
use betterlite_core::api::realtime_session_state::{ConnectionSetup, RealtimeStreaming};
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use serde_json::{Value, json};
use tokio::runtime::Builder;
use tokio::sync::Notify;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use super::Failure;
use super::direction::{ScriptedFailure, run_backend_direction, run_client_direction};
use super::failure;
use super::guardrail::ScriptedGuardrail;
use super::input::{
    CloseDocument, GuardrailDocument, Input, LifetimeDocument, Mode, SetupDocument,
};
use super::provider::NoProvider;
use super::transport::{
    ClientData, Closes, Frames, Locked as _, ScriptedBackend, ScriptedClient, recorded_closes,
    recorded_frames,
};

const DEADLINE: Duration = Duration::from_secs(120);

struct Probe {
    state: RealtimeStreaming,
    guardrails: Vec<ScriptedGuardrail>,
    redactor: Redactor,
    client: Frames,
    closes: Closes,
    backend: Frames,
    applied: Arc<Mutex<Vec<Vec<String>>>>,
    timeline: Frames,
}

impl Probe {
    fn new(input: &Input) -> Result<Self, Failure> {
        let (client, closes, client_data) = scripted_client(input);
        let backend: Frames = Arc::new(Mutex::new(Vec::new()));
        let state = streaming_state(input, client_data, Arc::clone(&backend));
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
            .map_err(|error| failure("RedactionError", error.to_string()))?;
        let applied: Arc<Mutex<Vec<Vec<String>>>> = Arc::new(Mutex::new(Vec::new()));
        let guardrails = scripted_guardrails(&input.guardrails, &applied);
        Ok(Self {
            state,
            guardrails,
            redactor,
            client,
            closes,
            backend,
            applied,
            timeline: Arc::new(Mutex::new(Vec::new())),
        })
    }
}

fn scripted_client(input: &Input) -> (Frames, Closes, Arc<Mutex<ClientData>>) {
    let client_data = Arc::new(Mutex::new(ClientData::default()));
    let (client, closes) = {
        let data = client_data.locked();
        (Arc::clone(&data.log), Arc::clone(&data.closes))
    };
    {
        let mut data = client_data.locked();
        data.incoming = input.messages.iter().cloned().collect();
        data.failures.clone_from(&input.client_send_failures);
        data.close_failures.clone_from(&input.client_close_failures);
    }
    (client, closes, client_data)
}

fn streaming_state(
    input: &Input,
    client_data: Arc<Mutex<ClientData>>,
    backend: Frames,
) -> RealtimeStreaming {
    let mut state = RealtimeStreaming::new(ConnectionSetup {
        client: Some(Box::new(ScriptedClient {
            data: client_data,
            scope: WebSocketScope::default(),
        })),
        backend: Some(Box::new(ScriptedBackend {
            log: backend,
            failures: Vec::new(),
            attempts: 0,
        })),
        logging: None,
        logging_worker: None,
        provider_config: None,
        event_normalizer: None,
        logged_real_time_event_types: LoggedEventTypes::All,
        model: String::from("scripted"),
        user_api_key_dict: None,
        request_data: None,
        force_transcription_model: None,
        client_wants_beta: false,
        backend_uses_beta_protocol: Some(input.beta_backend),
    });
    configure_setup(&mut state, &input.setup);
    state
}

fn configure_setup(state: &mut RealtimeStreaming, setup: &SetupDocument) {
    state.setup.backend_setup_complete.0 = setup.backend_setup_complete.unwrap_or(true);
    state.setup.flushing_pending_messages.0 = setup.flushing;
    state.setup.turn_detection_update_sent.0 = setup.turn_detection_sent;
    state
        .setup
        .pending_guardrail_message
        .clone_from(&setup.pending_guardrail_message);
    state
        .response
        .session_configuration_request
        .clone_from(&setup.session_configuration_request);
    state
        .setup
        .pending_messages_until_setup
        .clone_from(&setup.pending);
    state.setup.pending_messages_byte_total = setup
        .pending
        .iter()
        .map(String::len)
        .fold(0_usize, usize::saturating_add);
}

fn scripted_guardrails(
    documents: &[GuardrailDocument],
    applied: &Arc<Mutex<Vec<Vec<String>>>>,
) -> Vec<ScriptedGuardrail> {
    documents
        .iter()
        .map(|document| {
            ScriptedGuardrail::new(
                document.hooks.clone(),
                document.block_on.clone(),
                Arc::clone(applied),
            )
        })
        .collect()
}

fn references(guardrails: &[ScriptedGuardrail]) -> Vec<&dyn RealtimeGuardrail<Error = Infallible>> {
    let mut refs = Vec::with_capacity(guardrails.len());
    for guardrail in guardrails {
        let reference: &dyn RealtimeGuardrail<Error = Infallible> = guardrail;
        refs.push(reference);
    }
    refs
}

async fn ack(probe: &mut Probe) -> ClientLoopExit {
    let refs = references(&probe.guardrails);
    let mut relay: RealtimeClientRelay<'_, NoProvider, Infallible> = RealtimeClientRelay {
        state: &mut probe.state,
        provider: None,
        guardrails: &refs,
        callbacks: relay_callbacks(&refs),
        redactor: Some(&probe.redactor),
    };
    relay.client_ack_messages().await
}

fn lifetime_documents(input: &Input) -> Result<(&LifetimeDocument, &CloseDocument), Failure> {
    let Some(lifetime) = input.lifetime.as_ref() else {
        return Err(failure(
            "ValueError",
            String::from("a lifetime run needs a lifetime script"),
        ));
    };
    let Some(close) = input.close.as_ref() else {
        return Err(failure(
            "ValueError",
            String::from("a lifetime run needs a close document"),
        ));
    };
    Ok((lifetime, close))
}

async fn lifetime_run(probe: &mut Probe, input: &Input) -> HelperResult {
    let (lifetime, close) = lifetime_documents(input)?;
    let cancellation = CancellationToken::new();
    let notification = Arc::new(Notify::new());
    let client_direction = run_client_direction(
        Arc::clone(&probe.timeline),
        cancellation.clone(),
        Arc::clone(&notification),
        lifetime,
    );
    let backend_direction = run_backend_direction(
        Arc::clone(&probe.timeline),
        cancellation.clone(),
        Arc::clone(&notification),
        lifetime,
        BackendClose {
            code: close.code,
            reason: close.reason.clone(),
        },
    );
    let refs = references(&probe.guardrails);
    let mut relay: RealtimeClientRelay<'_, NoProvider, Infallible> = RealtimeClientRelay {
        state: &mut probe.state,
        provider: None,
        guardrails: &refs,
        callbacks: relay_callbacks(&refs),
        redactor: Some(&probe.redactor),
    };
    match relay
        .bidirectional_forward(client_direction, backend_direction, &cancellation)
        .await
    {
        Ok(()) => Ok(json!({"ok": true})),
        Err(ScriptedFailure::Backend) => Ok(json!({"error": "backend"})),
    }
}

fn snapshot(
    probe: &Probe,
    input: &Input,
    exit: Option<ClientLoopExit>,
    result: Option<Value>,
) -> Value {
    let mode = match input.mode {
        Mode::Ack => "ack",
        Mode::Lifetime => "lifetime",
    };
    let exit_name = exit.map(|value| match value {
        ClientLoopExit::ClientDisconnected => "client_disconnected",
        ClientLoopExit::BackendClosed => "backend_closed",
    });
    json!({
        "mode": mode,
        "exit": exit_name,
        "result": result,
        "client": recorded_frames(&probe.client),
        "backend": recorded_frames(&probe.backend),
        "closes": recorded_closes(&probe.closes),
        "pending": probe.state.setup.pending_messages_until_setup,
        "applied": probe.applied.locked().clone(),
        "timeline": probe.timeline.locked().clone(),
        "flags": {
            "turn_detection_sent": probe.state.setup.turn_detection_update_sent.0,
            "setup_complete": probe.state.setup.backend_setup_complete.0,
            "flushing": probe.state.setup.flushing_pending_messages.0,
            "pending_guardrail": probe.state.setup.pending_guardrail_message,
        },
    })
}

async fn observe(input: &Input) -> HelperResult {
    let mut probe = Probe::new(input)?;
    let (exit, result) = match input.mode {
        Mode::Ack => (Some(ack(&mut probe).await), None),
        Mode::Lifetime => (None, Some(lifetime_run(&mut probe, input).await?)),
    };
    Ok(snapshot(&probe, input, exit, result))
}

pub(crate) fn drive(input: &Input) -> HelperResult {
    input
        .check()
        .map_err(|message| failure("ValueError", message))?;
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failure("OSError", error.to_string()))?;
    runtime
        .block_on(async { timeout(DEADLINE, observe(input)).await })
        .map_err(|_elapsed| {
            failure(
                "TimeoutError",
                String::from("the helper ran past its deadline"),
            )
        })?
}
