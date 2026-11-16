use std::collections::{BTreeMap, VecDeque};

use betterlite_openai::chat::ChatRequest;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::ChatExecutor;
use betterlite_openai::stream::SseParser;
use futures_util::StreamExt as _;
use serde::Serialize;

use super::error::SdkHttpError;
use super::result::{SdkResult, SyncRun};
use super::{ChatMessage, OpenaiChat, native};
use call::MessagesSyncStreamCall;

pub mod call;
mod diagnostics;
mod run;
mod source;

#[derive(Serialize)]
struct WireChat<'wire> {
    model: &'wire str,
    messages: &'wire [ChatMessage],
    stream: bool,
}

pub(crate) async fn send(
    call: &MessagesSyncStreamCall,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let executor = executor(call, api_base, env)?;
    let deployment = executor
        .deployments
        .first()
        .ok_or_else(|| SdkHttpError::Invalid(format!("no deployment serves {}", call.model)))?;
    let mut runs = Vec::with_capacity(call.runs.len());
    for spec in &call.runs {
        let payloads = prefetch(&executor, deployment, call).await?;
        runs.push(run::run(call, spec, payloads)?);
    }
    Ok(SdkResult::MessagesSyncStream {
        value: SyncRun { runs },
    })
}

fn executor(
    call: &MessagesSyncStreamCall,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<ChatExecutor, SdkHttpError> {
    // the same deployment the native chat call builds, so base URLs, keys and timeouts resolve alike
    let chat = OpenaiChat {
        model: call.model.clone(),
        messages: call.messages.clone(),
        api_key: call.api_key.clone(),
        timeout_seconds: call.timeout_seconds,
    };
    native::executor(&chat, api_base, env)
}

// One POST per run, with the body the synchronous wrapper would send for the same case
async fn prefetch(
    executor: &ChatExecutor,
    deployment: &ChatDeployment,
    call: &MessagesSyncStreamCall,
) -> Result<VecDeque<Vec<u8>>, SdkHttpError> {
    let wire = WireChat {
        model: &call.model,
        messages: &call.messages,
        stream: true,
    };
    let body = serde_json::to_vec(&wire).map_err(SdkHttpError::Serialize)?;
    let request =
        ChatRequest::parse(&body).map_err(|source| SdkHttpError::ChatParse(Box::new(source)))?;
    let response = executor
        .stream(deployment, &request)
        .await
        .map_err(|error| SdkHttpError::Chat(Box::new(error)))?;
    let mut stream = response.body_stream();
    let mut parser = SseParser::default();
    let mut payloads = VecDeque::new();
    while let Some(bytes) = stream.next().await {
        let bytes = bytes.map_err(|source| SdkHttpError::Stream(Box::new(source)))?;
        for event in parser
            .push(&bytes)
            .map_err(|error| SdkHttpError::Invalid(error.to_string()))?
        {
            payloads.push_back(event);
        }
    }
    for event in parser.finish() {
        payloads.push_back(event);
    }
    Ok(payloads)
}
