use std::sync::Arc;

use betterlite_prompts::factory::PromptFactory;
use betterlite_provider::replicate::poll::completion::{Replicate, ReplicateCall};
use betterlite_provider::replicate::poll::stream::poll_stream;
use betterlite_provider::replicate::poll::{OutputPolicy, ReplicateCompletion, ReplicatePolling};
use chrono::DateTime;
use futures_util::StreamExt as _;
use serde_json::{Value, json};

use super::ops::{failure_output, polling_for, usage_value};
use super::script::{
    RecordingSleeper, ScriptedTransport, SilentLogger, StandInCounter, factory, header_map, runtime,
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
}

impl Fixture {
    fn new() -> Self {
        Self {
            prompts: factory(),
            counter: StandInCounter,
            logger: SilentLogger,
        }
    }

    fn client(
        &self,
        scripted: &ScriptedTransport,
        sleeper: &RecordingSleeper,
        polling: ReplicatePolling,
        policy: OutputPolicy,
    ) -> Replicate<'_> {
        Replicate {
            transport: Arc::new(scripted.clone()),
            sleeper: Arc::new(sleeper.clone()),
            prompts: &self.prompts,
            counter: &self.counter,
            logger: &self.logger,
            polling,
            policy,
        }
    }
}

fn poll_outcome(
    runtime: &tokio::runtime::Runtime,
    fixture: &Fixture,
    case: &PollCase,
) -> Result<Value, Box<HelperError>> {
    let scripted = ScriptedTransport::new(case.scripted());
    let sleeper = RecordingSleeper::default();
    let client = fixture.client(
        &scripted,
        &sleeper,
        polling_for(case.retries, case.retry_delay, 0.5_f64),
        OutputPolicy::Strict,
    );
    let mut messages = case.messages.clone();
    let mut optional_params = case.optional_params.clone();
    let result = runtime.block_on(client.complete(ReplicateCall {
        model: &case.model,
        messages: &mut messages,
        optional_params: &mut optional_params,
        defaults: &case.defaults,
        custom_prompts: &case.custom_prompt_dict,
        api_key: Some("key"),
        now: DateTime::from_timestamp(case.now, 0).unwrap_or_default(),
    }));
    let mut output = match result {
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
    };
    append_script(&mut output, &scripted, &sleeper)?;
    Ok(output)
}

pub(super) fn poll(cases: &[PollCase]) -> HelperResult {
    let runtime = runtime()?;
    let fixture = Fixture::new();
    let mut results = Vec::with_capacity(cases.len());
    for case in cases {
        results.push(poll_outcome(&runtime, &fixture, case)?);
    }
    Ok(json!({"requests": results}))
}

fn stream_outcome(
    runtime: &tokio::runtime::Runtime,
    fixture: &Fixture,
    case: &StreamCase,
) -> Result<Value, Box<HelperError>> {
    let scripted = ScriptedTransport::new(case.polls.clone());
    let sleeper = RecordingSleeper::default();
    let policy = match case.policy {
        Policy::Strict => OutputPolicy::Strict,
        Policy::Lenient => OutputPolicy::Lenient,
    };
    let client = fixture.client(
        &scripted,
        &sleeper,
        polling_for(5, 1.0_f64, case.polling_delay),
        policy,
    );
    let headers = header_map(&case.headers)?;
    let mut stream = poll_stream(&client, case.url.clone(), headers);
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
    Ok(json!({
        "delays": sleeper.delays(),
        "chunks": chunks,
        "requests": scripted.requests_output()?,
        "error": error,
    }))
}

pub(super) fn streams(cases: &[StreamCase]) -> HelperResult {
    let runtime = runtime()?;
    let fixture = Fixture::new();
    let mut results = Vec::with_capacity(cases.len());
    for case in cases {
        results.push(stream_outcome(&runtime, &fixture, case)?);
    }
    Ok(json!({"streams": results}))
}
