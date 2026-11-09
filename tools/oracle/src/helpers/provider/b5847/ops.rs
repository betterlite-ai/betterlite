use std::sync::Arc;

use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::factory::PromptFactory;
use betterlite_provider::replicate::error::ReplicateFailure;
use betterlite_provider::replicate::params::{SUPPORTED_OPENAI_PARAMS, map_openai_params};
use betterlite_provider::replicate::poll::completion::{Replicate, ReplicateCall};
use betterlite_provider::replicate::poll::stream::poll_stream;
use betterlite_provider::replicate::poll::{OutputPolicy, ReplicateCompletion, ReplicatePolling};
use betterlite_provider::replicate::reply::{ReplicateUsage, ReplyInput, transform_response};
use betterlite_provider::replicate::request::{
    ReplicateBody, RequestContext, RequestInput, get_complete_url, transform_request,
    validate_environment,
};
use chrono::DateTime;
use futures_util::StreamExt as _;
use serde_json::{Value, json};

use super::script::{
    FIXED_NOW, RecordingSleeper, ScriptedTransport, SilentLogger, StandInCounter, factory,
    header_map, headers_output, helper_error, runtime,
};
use super::{HeaderRequest, MapCase, Policy, PollCase, ReplyCase, RequestCase, StreamCase};
use crate::helpers::{HelperError, HelperResult};

fn polling_for(
    retries: i64,
    retry_delay_seconds: f64,
    polling_delay_seconds: f64,
) -> ReplicatePolling {
    ReplicatePolling {
        retries,
        retry_delay_seconds,
        polling_delay_seconds,
        model_name_with_id_length: 64,
    }
}

fn failure_output(failure: &ReplicateFailure) -> Value {
    match failure.api_message() {
        Some(message) => json!({
            "ok": false,
            "status": failure.status_code(),
            "message": message,
        }),
        None => json!({
            "ok": false,
            "status": failure.status_code(),
            "message": failure.to_string(),
        }),
    }
}

fn usage_value(usage: &ReplicateUsage) -> Value {
    json!({
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
    })
}

#[must_use]
pub(super) fn map_params(cases: &[MapCase]) -> Value {
    let mut results = Vec::with_capacity(cases.len());
    for case in cases {
        let mut optional_params = case.optional_params.clone();
        map_openai_params(&case.non_default, &mut optional_params, &case.model);
        results.push(json!({"options": optional_params}));
    }
    json!({"cases": results, "supported": SUPPORTED_OPENAI_PARAMS})
}

#[must_use]
pub(super) fn urls(models: &[String]) -> Value {
    let mut found = Vec::with_capacity(models.len());
    for model in models {
        found.push(get_complete_url(model));
    }
    json!({"urls": found})
}

pub(super) fn headers(requests: &[HeaderRequest]) -> HelperResult {
    let mut found = Vec::with_capacity(requests.len());
    for request in requests {
        let incoming = header_map(&request.headers)?;
        let headers = validate_environment(&incoming, request.api_key.as_deref())
            .map_err(|failure| helper_error("ReplicateError", failure))?;
        found.push(headers_output(&headers)?);
    }
    Ok(json!({"headers": found}))
}

fn request_outcome(
    runtime: &tokio::runtime::Runtime,
    prompts: &PromptFactory,
    polling: &ReplicatePolling,
    case: &RequestCase,
) -> Value {
    let mut messages = case.messages.clone();
    let mut optional_params = case.optional_params.clone();
    let outcome = runtime.block_on(transform_request(
        RequestInput {
            model: &case.model,
            messages: &mut messages,
            optional_params: &mut optional_params,
            defaults: &case.defaults,
            custom_prompts: &case.custom_prompt_dict,
        },
        &RequestContext {
            prompts,
            polling,
            now: DateTime::from_timestamp(FIXED_NOW, 0).unwrap_or_default(),
        },
    ));
    match outcome {
        Ok(body) => json!({
            "ok": true,
            "body": body,
            "messages": messages,
            "options": optional_params,
        }),
        Err(failure) => failure_output(&failure),
    }
}

pub(super) fn requests(cases: &[RequestCase]) -> HelperResult {
    let runtime = runtime()?;
    let prompts = factory();
    let polling = polling_for(5, 1.0_f64, 0.5_f64);
    let mut results = Vec::with_capacity(cases.len());
    for case in cases {
        results.push(request_outcome(&runtime, &prompts, &polling, case));
    }
    Ok(json!({"requests": results}))
}

fn reply_outcome(
    runtime: &tokio::runtime::Runtime,
    counter: &StandInCounter,
    logger: &SilentLogger,
    case: &ReplyCase,
) -> Result<Value, Box<HelperError>> {
    let headers = header_map(&case.headers)?;
    let request_data = ReplicateBody {
        input: Object::new(),
        version: None,
    };
    let input = ReplyInput {
        model: &case.model,
        body: case.body.as_bytes(),
        headers: &headers,
        messages: &case.messages,
        request_data: &request_data,
        api_key: None,
    };
    match runtime.block_on(transform_response(&input, counter, logger)) {
        Ok(reply) => Ok(json!({
            "ok": true,
            "model": reply.model,
            "text": reply.text,
            "usage": usage_value(&reply.usage),
        })),
        Err(failure) => Ok(failure_output(&failure)),
    }
}

pub(super) fn replies(cases: &[ReplyCase]) -> HelperResult {
    let runtime = runtime()?;
    let counter = StandInCounter;
    let logger = SilentLogger;
    let mut results = Vec::with_capacity(cases.len());
    for case in cases {
        results.push(reply_outcome(&runtime, &counter, &logger, case)?);
    }
    Ok(json!({"replies": results}))
}

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
