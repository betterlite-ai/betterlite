mod fakes;
mod fixtures;
mod handler;
mod support;

use std::mem::take;
use std::sync::{Arc, Mutex};

use betterlite_core::api::responses_ws_history::{FrameSink, ManagedResponsesWebSocket};
use betterlite_core::api::responses_ws_managed::provider::ResponsesStreaming;
use betterlite_core::api::responses_ws_managed::run::{FrameSource, run};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_oracle_runner::routing_mock_scripts::ProviderCall;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;
use tracing::instrument::WithSubscriber as _;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::registry;

use self::fakes::ScriptedProvider;
use self::handler::{Captured, build_handler};
use self::support::{QuotaRow, ScriptRow, ScriptedSource, Shared, SinkRow, Socket, lock};

#[derive(Deserialize)]
struct Setup {
    model: String,
    litellm_metadata: Option<IndexMap<String, Json>>,
    custom_llm_provider: Option<String>,
    api_key: Option<String>,
    api_base: Option<String>,
    timeout: Option<f64>,
    first_message: Option<String>,
    kwargs: IndexMap<String, Json>,
}

#[derive(Deserialize)]
pub(crate) struct Input {
    setup: Setup,
    frames: Vec<String>,
    disconnect: String,
    scripts: Vec<ScriptRow>,
    quota: Option<QuotaRow>,
    sink: SinkRow,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|source| support::helper_error("RuntimeError", source.to_string()))?;
        let shared = Shared::default();
        let mock = fixtures::start_mock(input.frames.len().saturating_add(2))
            .map_err(|error| support::helper_error("MockError", error.to_string()))?;
        let provider = ScriptedProvider {
            shared: Arc::clone(&shared),
            scripts: Mutex::new(input.scripts.iter().cloned().collect()),
            outbound: fixtures::outbound()
                .map_err(|error| support::helper_error("FixtureError", error.to_string()))?,
            base_url: mock.base_url.clone(),
        };
        let answer = runtime.block_on(run_transcript(input, &shared, &provider))?;
        let (calls, failure) = mock
            .finish_recording()
            .map_err(|error| support::helper_error("MockError", error.to_string()))?;
        if let Some(error) = failure {
            return Err(support::helper_error("MockError", error.to_string()));
        }
        answer_with_requests(answer, &calls)
    }
}

async fn run_transcript(
    input: &Input,
    shared: &Shared,
    provider: &ScriptedProvider,
) -> HelperResult {
    let socket = Socket {
        shared: Arc::clone(shared),
        fails_at: input.sink.fails_at,
        message: input.sink.message.clone(),
    };
    let source = ScriptedSource {
        shared: Arc::clone(shared),
        frames: input.frames.clone(),
        disconnect: input.disconnect.clone(),
        next: 0,
    };
    let mut handler = build_handler(&input.setup, socket, shared, input.quota.as_ref());
    let captured = Captured::default();
    let subscriber = registry().with(captured.clone());
    transcript(&mut handler, provider, source, shared, &captured)
        .with_subscriber(subscriber)
        .await
}

fn answer_with_requests(answer: Value, calls: &[ProviderCall]) -> HelperResult {
    let Value::Object(mut fields) = answer else {
        return Err(support::helper_error(
            "AnswerShape",
            String::from("the transcript answer is not an object"),
        ));
    };
    fields.insert(String::from("provider_requests"), fixtures::requests(calls));
    Ok(Value::Object(fields))
}

async fn transcript<Sink, Provider, Source>(
    handler: &mut ManagedResponsesWebSocket<Sink>,
    provider: &Provider,
    source: Source,
    shared: &Shared,
    captured: &Captured,
) -> HelperResult
where
    Sink: FrameSink,
    Provider: ResponsesStreaming,
    Source: FrameSource,
{
    run(handler, provider, source).await;
    let frame_rows = support::frames(shared)?;
    let (provider_calls, quota_calls) = support::recorded(shared);
    Ok(json!({
        "frames": frame_rows,
        "provider_calls": provider_calls,
        "quota_calls": quota_calls,
        "history": support::history(handler),
        "trace": json!(lock(shared).trace),
        "diagnostics": json!(take(&mut *lock(&captured.0))),
    }))
}
