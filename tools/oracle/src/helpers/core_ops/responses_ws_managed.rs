#[path = "responses_ws_managed/fixtures/fakes.rs"]
mod fakes;
#[path = "responses_ws_managed/fixtures.rs"]
mod fixtures;
#[path = "responses_ws_managed/handler.rs"]
mod handler;
#[path = "responses_ws_managed/support.rs"]
mod support;

use std::collections::BTreeMap;
use std::mem::take;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

use betterlite_core::api::responses_ws_managed::run::run;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_oracle_runner::mock::MockProvider;
use betterlite_oracle_runner::routing_mock_scripts::ProviderCall;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};
use tracing::instrument::WithSubscriber as _;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::registry;

use self::fakes::ScriptedProvider;
use self::handler::{Captured, build_handler};
use self::support::{ScriptedSource, Shared, Socket, lock};
#[path = "responses_ws_managed/input.rs"]
pub(crate) mod input;
#[path = "responses_ws_managed/fixtures/input.rs"]
mod input_schema;
#[path = "responses_ws_managed/fixtures/input/tests.rs"]
mod input_tests;
use self::input::{Input, Setup};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = runtime_for(input)?;
        let shared = Shared::default();
        let (mock, provider) = start_provider(input, &shared)?;
        let answer = runtime
            .block_on(async {
                timeout(
                    Duration::from_millis(input.deadline_millis),
                    run_transcript(input, &shared, &provider),
                )
                .await
            })
            .map_err(|source| {
                Box::new(HelperError {
                    class: String::from("TimeoutError"),
                    message: source.to_string(),
                    status: None,
                    error_type: Some(None),
                    param: Some(None),
                    code: Some(None),
                })
            })??;
        finish_mock(answer, mock)
    }
}

fn runtime_for(input: &Input) -> Result<Runtime, Box<HelperError>> {
    input
        .check_bounds(1024, 0x0010_0000, 0x0100_0000)
        .map_err(|source| {
            Box::new(HelperError {
                class: String::from("InputError"),
                message: source.to_string(),
                status: None,
                error_type: Some(None),
                param: Some(None),
                code: Some(None),
            })
        })?;
    Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| {
            Box::new(HelperError {
                class: String::from("RuntimeError"),
                message: source.to_string(),
                status: None,
                error_type: Some(None),
                param: Some(None),
                code: Some(None),
            })
        })
}

fn start_provider(
    input: &Input,
    shared: &Shared,
) -> Result<(MockProvider, ScriptedProvider), Box<HelperError>> {
    let mock = fixtures::start_mock(input).map_err(|error| {
        Box::new(HelperError {
            class: String::from("MockError"),
            message: error.to_string(),
            status: None,
            error_type: Some(None),
            param: Some(None),
            code: Some(None),
        })
    })?;
    let provider = ScriptedProvider {
        shared: Arc::clone(shared),
        outbound: fixtures::outbound().map_err(|error| {
            Box::new(HelperError {
                class: String::from("FixtureError"),
                message: error.to_string(),
                status: None,
                error_type: Some(None),
                param: Some(None),
                code: Some(None),
            })
        })?,
        base_url: mock.base_url.clone(),
    };
    Ok((mock, provider))
}

fn finish_mock(answer: Value, mock: MockProvider) -> HelperResult {
    let (calls, failure) = mock.finish_recording().map_err(|error| {
        Box::new(HelperError {
            class: String::from("MockError"),
            message: error.to_string(),
            status: None,
            error_type: Some(None),
            param: Some(None),
            code: Some(None),
        })
    })?;
    if let Some(error) = failure {
        return Err(Box::new(HelperError {
            class: String::from("MockError"),
            message: error.to_string(),
            status: None,
            error_type: Some(None),
            param: Some(None),
            code: Some(None),
        }));
    }
    answer_with_requests(answer, &calls)
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
    run(&mut handler, provider, source)
        .with_subscriber(registry().with(captured.clone()))
        .await;
    let (provider_calls, quota_calls) = {
        let recorder = lock(shared);
        (json!(recorder.calls), json!(recorder.quotas))
    };
    let history = handler.session_history.iter().collect::<BTreeMap<_, _>>();
    Ok(json!({
        "frames": support::frames(shared)?,
        "provider_calls": provider_calls,
        "quota_calls": quota_calls,
        "history": history,
        "trace": json!(lock(shared).trace),
        "diagnostics": json!(take(&mut *lock(&captured.0))),
    }))
}

fn answer_with_requests(answer: Value, calls: &[ProviderCall]) -> HelperResult {
    let Value::Object(mut fields) = answer else {
        return Err(Box::new(HelperError {
            class: String::from("AnswerShape"),
            message: String::from("the transcript answer is not an object"),
            status: None,
            error_type: Some(None),
            param: Some(None),
            code: Some(None),
        }));
    };
    fields.insert(String::from("provider_requests"), fixtures::requests(calls));
    Ok(Value::Object(fields))
}
