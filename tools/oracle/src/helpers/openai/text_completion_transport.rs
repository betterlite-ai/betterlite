use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::text_completion_transport::TextCompletionTransport;
use betterlite_openai::text_completion_transport::call::{
    TextCompletionOutcome, TextCompletionRequest,
};
use betterlite_openai::text_completion_transport::error::{
    ErrorStatus, TextCompletionError, TextCompletionFailure,
};
use betterlite_openai::text_completion_transport::logging::SecretHeaders;
use betterlite_openai::text_completion_transport::sdk::{ClientKind, SdkError};
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;
use futures_util::StreamExt as _;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::helpers::{HelperError, HelperResult};
use fixture::{Answer, Constructor, Log, Recorder, Sdk, configs, instant};

mod fixture;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    model: Option<String>,
    messages: Option<Vec<Value>>,
    api_key: Option<String>,
    api_base: Option<String>,
    timeout: f64,
    organization: Option<String>,
    #[serde(default)]
    optional_params: Map<String, Value>,
    headers: Option<BTreeMap<String, String>>,
    acompletion: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Call {
    provider: ProviderId,
    #[serde(default)]
    supplied: bool,
    request: Request,
    answer: Answer,
}

#[derive(Serialize)]
struct Raised {
    mapped: bool,
    status: Option<u16>,
    message: String,
    headers: Option<BTreeMap<String, String>>,
}

#[derive(Serialize, Default)]
struct Output {
    result: Option<Value>,
    hidden_original_response: Option<Value>,
    chunks: Option<Vec<Value>>,
    raised: Option<Raised>,
    pre_call: Vec<Value>,
    post_call: Vec<Value>,
    built: Vec<Value>,
    calls: Vec<Value>,
}

impl Raised {
    fn new(
        mapped: bool,
        status: Option<u16>,
        message: String,
        headers: BTreeMap<String, String>,
    ) -> Self {
        Self {
            mapped,
            status,
            message,
            headers: Some(headers).filter(|fields| !fields.is_empty()),
        }
    }
}

fn sdk_raised(error: &SdkError) -> Raised {
    let (status, headers) = match error {
        SdkError::Status(failure) => (
            failure.status,
            SecretHeaders::from_response(&failure.headers).0,
        ),
        SdkError::Decode(_) | SdkError::Sse(_) | SdkError::Other { .. } => (None, BTreeMap::new()),
    };
    Raised::new(false, status, error.to_string(), headers)
}

fn wrapped_raised(error: &TextCompletionError) -> Raised {
    let status = match error.status {
        ErrorStatus::Missing => Some(500),
        ErrorStatus::Null => None,
        ErrorStatus::Code(code) => Some(code),
    };
    let headers = error.headers.as_ref().map(|fields| fields.0.clone());
    Raised::new(
        true,
        status,
        error.message.clone(),
        headers.unwrap_or_default(),
    )
}

fn project(chunk: &StreamChunk) -> Value {
    let choice = chunk.choices.first();
    json!({
        "model": chunk.model,
        "content": choice.and_then(|entry| entry.delta.content.as_ref()),
        "finish_reason": choice.and_then(|entry| entry.finish_reason.as_ref()),
    })
}

async fn flow(
    transport: &TextCompletionTransport<Constructor>,
    call: &Call,
    supplied: Option<&Sdk>,
    logger: &mut Recorder,
    output: &mut Output,
) -> Result<(), TextCompletionFailure> {
    let fields = &call.request;
    let request = TextCompletionRequest {
        model: fields.model.clone(),
        messages: fields.messages.clone(),
        api_key: fields.api_key.clone().map(SecretString::from),
        api_base: fields.api_base.clone(),
        timeout: fields.timeout,
        organization: fields.organization.clone(),
        optional_params: fields.optional_params.clone(),
        headers: fields.headers.clone().map(SecretHeaders),
        acompletion: fields.acompletion,
    };
    let begun = transport.begin(request, call.provider, logger)?;
    let sent = transport.send(&begun, supplied).await?;
    match transport.finish(&begun, sent, logger)? {
        TextCompletionOutcome::Buffered(reply) => {
            output.result = Some(reply.response);
            output.hidden_original_response = reply
                .hidden_original_response
                .as_deref()
                .map(serde_json::from_str)
                .transpose()?;
        }
        TextCompletionOutcome::Stream(mut chunks) => {
            let mut seen = Vec::new();
            while let Some(item) = chunks.next().await {
                match item {
                    Ok(chunk) => seen.push(project(&chunk)),
                    Err(error) => {
                        output.raised = Some(wrapped_raised(&error));
                        break;
                    }
                }
            }
            output.chunks = Some(seen);
        }
    }
    Ok(())
}

fn failure_raised(failure: TextCompletionFailure) -> Raised {
    match failure {
        TextCompletionFailure::Wrapped(error) => wrapped_raised(&error),
        TextCompletionFailure::Raw(error) => sdk_raised(&error),
    }
}

async fn drive(call: &Call) -> Output {
    let constructor = Constructor {
        built: Log::default(),
        calls: Log::default(),
        answer: call.answer.clone(),
    };
    let kind = if call.request.acompletion {
        ClientKind::Async
    } else {
        ClientKind::Sync
    };
    let supplied = call.supplied.then(|| Sdk {
        kind,
        calls: Arc::clone(&constructor.calls),
        answer: call.answer.clone(),
    });
    let transport = TextCompletionTransport {
        constructor,
        configs: configs(),
        now: instant,
        next_id: Uuid::nil,
    };
    let mut logger = Recorder::default();
    let mut output = Output::default();
    let flowed = flow(
        &transport,
        call,
        supplied.as_ref(),
        &mut logger,
        &mut output,
    )
    .await;
    if let Err(failure) = flowed {
        output.raised = Some(failure_raised(failure));
    }
    output.pre_call = logger.pre;
    output.post_call = logger.post;
    (output.built, output.calls) = transport.constructor.logs();
    output
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Call;

    fn run(input: &Self::Input) -> HelperResult {
        let failed = |class: &str, message: String| {
            Box::new(HelperError {
                class: class.to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message,
            })
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|error| failed("RuntimeError", error.to_string()))?;
        serde_json::to_value(runtime.block_on(drive(input)))
            .map_err(|error| failed("InvalidOutput", error.to_string()))
    }
}

mod tests;
