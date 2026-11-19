use std::collections::BTreeMap;
use std::str;
use std::sync::{Arc, PoisonError};

use betterlite_core::api::messages_native_execute::entry::{
    MessagesEntryRequest, MessagesEntryResult, MessagesExecuteStream,
};
use betterlite_core::api::messages_native_execute::error::MessagesExecuteError;
use betterlite_core::api::messages_native_execute::pipeline::{MessagesEntryOutcome, execute};
use betterlite_core::api::messages_native_execute::seams::MessagesEntrySeams;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use secrecy::SecretString;

use super::Call;
use super::Outcome;
use super::dispatch::{dispatch_record, execute_dispatch, record_failure};
use super::scripted::Scripts;
use crate::sdk_http::error::SdkHttpError;
use crate::sdk_http::result::kinds::SdkResult;

pub(super) async fn send(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let base = api_base.strip_suffix("/v1").unwrap_or(api_base);
    let scripts = Scripts::new(call);
    let redactor =
        Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?);
    let mut outcome = Outcome::empty();
    match run_entry(call, base, &scripts, redactor).await {
        Ok(MessagesEntryOutcome::Returned(result)) => fill_returned(&mut outcome, result).await?,
        Ok(MessagesEntryOutcome::Dispatch(continuation)) => {
            outcome.dispatch = Some(dispatch_record(&continuation.dispatch));
            let sent = execute_dispatch(&continuation.dispatch, base, env).await?;
            match continuation.resume(sent) {
                Ok(result) => fill_returned(&mut outcome, result).await?,
                Err(error) => record_failure(&mut outcome, error)?,
            }
        }
        Err(error) => record_failure(&mut outcome, error)?,
    }
    let recordings = scripts
        .recordings
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    outcome.cache = recordings.cache.clone().unwrap_or_default();
    outcome.hooks = recordings.hooks.clone();
    drop(recordings);
    Ok(SdkResult::MessagesNativeExecute {
        value: Box::new(outcome),
    })
}

async fn run_entry(
    call: &Call,
    base: &str,
    scripts: &Scripts,
    redactor: Arc<Redactor>,
) -> Result<MessagesEntryOutcome, MessagesExecuteError> {
    let seams = MessagesEntrySeams {
        cache: &scripts.cache,
        provider_resolution: &scripts.resolver,
        pre_request_hooks: &scripts.hooks,
        search_short_circuits: &scripts.searches,
        interceptors: &scripts.interceptors,
        redactor,
    };
    execute(entry_request(call, base), &seams).await
}

fn entry_request(call: &Call, base: &str) -> MessagesEntryRequest {
    let request = &call.request;
    MessagesEntryRequest {
        max_tokens: request.max_tokens,
        messages: request.messages.clone(),
        model: request.model.clone(),
        metadata: request.metadata.clone(),
        stop_sequences: request.stop_sequences.clone(),
        stream: request.stream,
        system: request.system.clone(),
        temperature: request.temperature.clone(),
        thinking: request.thinking.clone(),
        tool_choice: request.tool_choice.clone(),
        tools: request.tools.clone(),
        top_k: request.top_k.clone(),
        top_p: request.top_p.clone(),
        api_key: request.api_key.clone().map(SecretString::from),
        api_base: Some(base.to_owned()),
        client: None,
        custom_llm_provider: request.custom_llm_provider.clone(),
        kwargs: request.kwargs.clone(),
    }
}

async fn fill_returned(
    outcome: &mut Outcome,
    result: MessagesEntryResult,
) -> Result<(), SdkHttpError> {
    match result {
        MessagesEntryResult::Response(value) => outcome.response = Some(value),
        MessagesEntryResult::None => {}
        MessagesEntryResult::Stream(stream) => drain_stream(outcome, stream).await?,
    }
    Ok(())
}

async fn drain_stream(
    outcome: &mut Outcome,
    mut stream: Box<dyn MessagesExecuteStream>,
) -> Result<(), SdkHttpError> {
    while let Some(chunk) = stream
        .next_chunk()
        .await
        .map_err(|error| SdkHttpError::Invalid(error.to_string()))?
    {
        let frame = str::from_utf8(&chunk).map_err(SdkHttpError::Utf8)?;
        if let Some(line) = frame.lines().next()
            && let Some(event) = line.strip_prefix("event: ")
        {
            outcome.stream.push(event.to_owned());
        }
    }
    Ok(())
}
