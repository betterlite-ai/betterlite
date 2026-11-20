use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_core::api::realtime_backend_relay::RealtimeBackendRelay;
use betterlite_core::api::realtime_backend_relay::relay::{
    BackendToClientRelay as _, FAILURE_LOGGED_KEY,
};
use betterlite_core::api::realtime_backend_relay::relay_callbacks;
use betterlite_core::api::realtime_backend_relay::source::{
    BackendRefusal, RealtimeFailureDispatch,
};
use betterlite_core::api::realtime_contracts::close::BackendClose;
use betterlite_core::api::realtime_contracts::transport::WebSocketScope;
use betterlite_core::api::realtime_guardrails::callback::RealtimeGuardrail;
use betterlite_core::api::realtime_session_state::logging::{
    RealtimeLogging, RealtimeSuccessQueue, SUCCESS_LOGGED_KEY, SessionInput, SuccessWork,
};
use betterlite_core::api::realtime_session_state::{
    ConnectionSetup, RealtimeProviderConfig, RealtimeStreaming,
};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::helper::HelperResult;
use indexmap::IndexMap;
use serde_json::{Value, json};
use tokio::runtime::Builder;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use super::Failure;
use super::failure;
use super::input::Input;
use super::provider::ScriptedProvider;
use super::transport::{ClientSink, Frames, Locked as _, RecordingBackend, ScriptedSource, frames};

const DEADLINE: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Default)]
struct Recorder {
    details: Arc<Mutex<Vec<(String, Json)>>>,
}

// LiteLLM's logging object only reports the two completion flags, and the queue admits every success work
impl RealtimeLogging for Recorder {
    fn pre_call(&self, _input: &SessionInput<'_>, _api_key: &str) {}

    fn record_model_call_detail(&self, key: &str, value: Json) {
        self.details.locked().push((key.to_owned(), value));
    }

    fn dispatch_success_handlers(&self, _messages: &[IndexMap<String, Json>]) -> SuccessWork {
        Box::pin(async {})
    }
}

impl RealtimeSuccessQueue for Recorder {
    fn enqueue(&self, _work: SuccessWork) -> bool {
        true
    }
}

impl RealtimeFailureDispatch for Recorder {
    fn dispatch_failure_handlers(
        &self,
        _refusal: &BackendRefusal,
        _prefer_async_handlers: bool,
    ) -> SuccessWork {
        Box::pin(async {})
    }
}

struct Probe {
    state: RealtimeStreaming,
    source: ScriptedSource,
    provider: Option<ScriptedProvider>,
    client: Frames,
    backend: Frames,
    recorder: Recorder,
}

impl Probe {
    fn new(input: &Input) -> Self {
        let client: Frames = Arc::new(Mutex::new(Vec::new()));
        let backend: Frames = Arc::new(Mutex::new(Vec::new()));
        let recorder = Recorder::default();
        let provider = input.provider.map(|document| ScriptedProvider {
            requires_session_configuration: document.requires_session_configuration,
        });
        let mut state =
            RealtimeStreaming::new(connection(input, provider, &client, &backend, &recorder));
        state.setup.pending_messages_until_setup = input.pending.clone();
        state.setup.pending_messages_byte_total = input
            .pending
            .iter()
            .map(String::len)
            .fold(0_usize, usize::saturating_add);
        Self {
            state,
            source: ScriptedSource {
                frames: input.frames.iter().cloned().collect(),
                close: input.close.clone(),
                decode_calls: Vec::new(),
            },
            provider,
            client,
            backend,
            recorder,
        }
    }

    fn snapshot(&self, close: &BackendClose) -> Value {
        let details = self.recorder.details.locked();
        json!({
            "close": {"code": close.code, "reason": close.reason},
            "client": frames(&self.client),
            "backend": frames(&self.backend),
            "decode_calls": self.source.decode_calls,
            "messages": records(&self.state.messages),
            "input_messages": records(&self.state.input_messages),
            "logged": {
                "success": detail(&details, SUCCESS_LOGGED_KEY),
                "failure": detail(&details, FAILURE_LOGGED_KEY),
            },
        })
    }
}

fn connection(
    input: &Input,
    provider: Option<ScriptedProvider>,
    client: &Frames,
    backend: &Frames,
    recorder: &Recorder,
) -> ConnectionSetup {
    ConnectionSetup {
        client: Some(Box::new(ClientSink {
            scope: WebSocketScope::default(),
            log: Arc::clone(client),
        })),
        backend: Some(Box::new(RecordingBackend {
            log: Arc::clone(backend),
        })),
        logging: Some(Box::new(recorder.clone())),
        logging_worker: Some(Box::new(recorder.clone())),
        provider_config: provider
            .map(|config| -> Box<dyn RealtimeProviderConfig> { Box::new(config) }),
        event_normalizer: None,
        logged_real_time_event_types: LoggedEventTypes::All,
        model: String::from("scripted"),
        user_api_key_dict: None,
        request_data: None,
        force_transcription_model: input.force_transcription_model.clone(),
        client_wants_beta: false,
        backend_uses_beta_protocol: None,
    }
}

fn records(messages: &[IndexMap<String, Json>]) -> Vec<Json> {
    messages
        .iter()
        .map(|message| Json::Object(message.clone()))
        .collect()
}

fn detail(details: &[(String, Json)], key: &str) -> Option<bool> {
    details.iter().find_map(|(name, value)| match value {
        Json::Bool(flag) if name == key => Some(*flag),
        Json::Bool(_)
        | Json::Null
        | Json::Number(_)
        | Json::Text(_)
        | Json::List(_)
        | Json::Object(_) => None,
    })
}

async fn observe(input: &Input) -> HelperResult {
    let mut probe = Probe::new(input);
    let guardrails: [&dyn RealtimeGuardrail<Error = Infallible>; 0] = [];
    let token = CancellationToken::new();
    let close = {
        let mut relay = RealtimeBackendRelay {
            state: &mut probe.state,
            provider: probe.provider.as_ref(),
            guardrails: &guardrails,
            callbacks: relay_callbacks(&guardrails),
            failure_dispatch: &probe.recorder,
        };
        relay
            .backend_to_client_send_messages(&mut probe.source, &token)
            .await
            .map_err(|error| failure("RealtimeRelayError", error.to_string()))?
    };
    Ok(probe.snapshot(&close))
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
