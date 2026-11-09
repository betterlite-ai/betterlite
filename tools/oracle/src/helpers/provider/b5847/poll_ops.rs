use std::sync::Arc;

use betterlite_prompts::factory::PromptFactory;
use betterlite_provider::replicate::error::ReplicateFailure;
use betterlite_provider::replicate::poll::completion::{Replicate, ReplicateCall};
use betterlite_provider::replicate::poll::stream::poll_stream;
use betterlite_provider::replicate::poll::transport::ReplicateTransport;
use betterlite_provider::replicate::poll::{
    OutputPolicy, ReplicateCompletion, ReplicatePolling, ReplicateSleeper, ReplicateStream,
};
use chrono::{DateTime, Utc};
use futures_util::StreamExt as _;
use serde_json::{Value, json};

use super::ops::{failure_output, polling_for, usage_value};
use super::script::{
    FIXED_NOW, FixedClock, RecordingSleeper, ScriptedResponse, ScriptedTransport, SilentLogger,
    StandInCounter, factory, header_map, runtime,
};
use super::{Policy, PollCase, StreamCase};
use crate::helpers::{HelperError, HelperResult};

fn append_script(
    output: &mut Value,
    transport: &ScriptedTransport,
    sleeper: &RecordingSleeper,
) -> Result<(), Box<HelperError>> {
    if let Value::Object(fields) = output {
        fields.insert(String::from("delays"), json!(sleeper.delays()));
        fields.insert(String::from("requests"), transport.requests_output()?);
    }
    Ok(())
}

struct Fixture {
    prompts: PromptFactory,
    counter: StandInCounter,
    logger: SilentLogger,
    clock: FixedClock,
}

impl Fixture {
    fn new(now: i64) -> Self {
        Self {
            prompts: factory(),
            counter: StandInCounter,
            logger: SilentLogger,
            clock: FixedClock(DateTime::<Utc>::from_timestamp(now, 0).unwrap_or_default()),
        }
    }

    fn client<Http, Sleep>(
        &self,
        transport: Arc<Http>,
        sleeper: Arc<Sleep>,
        polling: ReplicatePolling,
        policy: OutputPolicy,
    ) -> Replicate<'_>
    where
        Http: ReplicateTransport + 'static,
        Sleep: ReplicateSleeper + 'static,
    {
        Replicate {
            transport,
            sleeper,
            prompts: &self.prompts,
            counter: &self.counter,
            logger: &self.logger,
            clock: &self.clock,
            polling,
            policy,
        }
    }
}

struct Scripted<'deps> {
    client: Replicate<'deps>,
    recorder: Arc<ScriptedTransport>,
    sleeper: Arc<RecordingSleeper>,
}

fn scripted_client(
    fixture: &Fixture,
    responses: Vec<ScriptedResponse>,
    polling: ReplicatePolling,
    policy: OutputPolicy,
) -> Scripted<'_> {
    let recorder = Arc::new(ScriptedTransport::new(responses));
    let sleeper = Arc::new(RecordingSleeper::default());
    let client = fixture.client(Arc::clone(&recorder), Arc::clone(&sleeper), polling, policy);
    Scripted {
        client,
        recorder,
        sleeper,
    }
}

fn completion_value(result: Result<ReplicateCompletion, ReplicateFailure>) -> Value {
    match result {
        Ok(ReplicateCompletion::Reply(reply)) => json!({
            "ok": true,
            "model": reply.model,
            "text": reply.text,
            "usage": usage_value(&reply.usage),
            "created": reply.created,
        }),
        Ok(ReplicateCompletion::Stream(_)) => {
            json!({"ok": false, "status": null, "message": "the poll case created a stream"})
        }
        Err(failure) => failure_output(&failure),
    }
}

fn poll_outcome(
    runtime: &tokio::runtime::Runtime,
    fixture: &Fixture,
    case: &PollCase,
) -> Result<Value, Box<HelperError>> {
    let scripted = scripted_client(
        fixture,
        case.scripted(),
        polling_for(case.retries, case.retry_delay, 0.5_f64),
        OutputPolicy::Strict,
    );
    let mut messages = case.messages.clone();
    let mut optional_params = case.optional_params.clone();
    let result = runtime.block_on(scripted.client.complete(ReplicateCall {
        model: &case.model,
        messages: &mut messages,
        optional_params: &mut optional_params,
        defaults: &case.defaults,
        custom_prompts: &case.custom_prompt_dict,
        api_key: Some("key"),
        now: DateTime::from_timestamp(case.now, 0).unwrap_or_default(),
    }));
    let mut output = completion_value(result);
    append_script(&mut output, &scripted.recorder, &scripted.sleeper)?;
    Ok(output)
}

pub(super) fn poll(cases: &[PollCase]) -> HelperResult {
    let runtime = runtime()?;
    let mut results = Vec::with_capacity(cases.len());
    for case in cases {
        let fixture = Fixture::new(case.now);
        results.push(poll_outcome(&runtime, &fixture, case)?);
    }
    Ok(json!({"requests": results}))
}

fn collect_stream(
    runtime: &tokio::runtime::Runtime,
    mut stream: ReplicateStream,
) -> (Vec<Value>, Value) {
    let mut chunks = Vec::new();
    let mut error = Value::Null;
    while let Some(item) = runtime.block_on(stream.next()) {
        match item {
            Ok(chunk) => chunks.push(json!(chunk)),
            Err(failure) => {
                error = failure_output(&failure);
                break;
            }
        }
    }
    (chunks, error)
}

fn stream_setup<'deps>(
    fixture: &'deps Fixture,
    case: &StreamCase,
) -> Result<(Scripted<'deps>, ReplicateStream), Box<HelperError>> {
    let policy = match case.policy {
        Policy::Strict => OutputPolicy::Strict,
        Policy::Lenient => OutputPolicy::Lenient,
    };
    let headers = header_map(&case.headers)?;
    let scripted = scripted_client(
        fixture,
        case.polls.clone(),
        polling_for(5, 1.0_f64, case.polling_delay),
        policy,
    );
    let stream = poll_stream(&scripted.client, case.url.clone(), headers);
    Ok((scripted, stream))
}

fn stream_report(
    scripted: &Scripted<'_>,
    chunks: &[Value],
    error: &Value,
) -> Result<Value, Box<HelperError>> {
    Ok(json!({
        "delays": scripted.sleeper.delays(),
        "chunks": chunks,
        "requests": scripted.recorder.requests_output()?,
        "error": error,
    }))
}

fn stream_outcome(
    runtime: &tokio::runtime::Runtime,
    fixture: &Fixture,
    case: &StreamCase,
) -> Result<Value, Box<HelperError>> {
    let (scripted, stream) = stream_setup(fixture, case)?;
    let (chunks, error) = collect_stream(runtime, stream);
    stream_report(&scripted, &chunks, &error)
}

pub(super) fn streams(cases: &[StreamCase]) -> HelperResult {
    let runtime = runtime()?;
    let fixture = Fixture::new(FIXED_NOW);
    let streams: Vec<Value> = cases
        .iter()
        .map(|case| stream_outcome(&runtime, &fixture, case))
        .collect::<Result<_, _>>()?;
    Ok(json!({"streams": streams}))
}
