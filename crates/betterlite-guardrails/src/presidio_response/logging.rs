use std::thread;

use futures_util::future::join_all;
use serde_json::value::RawValue;
use tokio::runtime::Handle;

use crate::presidio_actions::check::CheckPiiCall;
use crate::presidio_actions::error::PresidioActionsError;
use crate::presidio_actions::request_data::{
    PresidioPerRequestConfig, RequestData, presidio_settings_from_request,
};
use crate::presidio_response::anthropic::process_anthropic_response;
use crate::presidio_response::error::PresidioResponseError;
use crate::presidio_response::log_merge::{append_log_entries, log_bucket_name, log_entries};
use crate::presidio_response::messages::{
    TextTask, apply_masked_texts, collect_text_tasks, request_messages, store_messages,
};
use crate::presidio_response::model::process_response_for_pii;
use crate::presidio_response::runtime::run_with_owned_client;
use crate::presidio_response::wire::{ChatMessage, ResponseBody, to_raw};
use crate::presidio_response::{HookContext, Mode};

const COMPLETION_CALL_TYPES: [&str; 2] = ["completion", "acompletion"];

type CheckOutcome = (Result<String, PresidioActionsError>, RequestData);

pub fn logging_hook(
    context: HookContext<'_>,
    request: &mut RequestData,
    response: &mut ResponseBody,
    call_type: &str,
) -> Result<(), PresidioResponseError> {
    if let Ok(handle) = Handle::try_current() {
        return thread::scope(|scope| {
            let join = scope.spawn(|| {
                handle.block_on(async_logging_hook(context, request, response, call_type))
            });
            join.join()
                .map_err(|_panic| PresidioResponseError::HookThreadPanicked)?
        });
    }
    run_with_owned_client(context, request, response, call_type)
}

pub async fn async_logging_hook(
    context: HookContext<'_>,
    request: &mut RequestData,
    response: &mut ResponseBody,
    call_type: &str,
) -> Result<(), PresidioResponseError> {
    if !COMPLETION_CALL_TYPES.contains(&call_type) {
        return Ok(());
    }
    let Some(mut messages) = request_messages(request)? else {
        return Ok(());
    };
    let masked = mask_request_messages(context, &mut messages, request).await?;
    if masked > 0 {
        store_messages(request, &messages)?;
    }
    match response {
        ResponseBody::Chat(chat)
            if chat
                .choices
                .as_ref()
                .is_some_and(|choices| !choices.is_empty()) =>
        {
            process_response_for_pii(context, chat, request, Mode::Mask).await
        }
        ResponseBody::Anthropic(message) => {
            process_anthropic_response(context, message, request, Mode::Mask).await
        }
        ResponseBody::Chat(_) | ResponseBody::Stream(_) | ResponseBody::Other(_) => Ok(()),
    }
}

async fn mask_request_messages(
    context: HookContext<'_>,
    messages: &mut [ChatMessage],
    request: &mut RequestData,
) -> Result<usize, PresidioResponseError> {
    let presidio_config = presidio_settings_from_request(request)?;
    let tasks = collect_text_tasks(messages);
    if tasks.is_empty() {
        return Ok(0);
    }
    let bucket = log_bucket_name(request);
    let gathered = run_checks(context, &tasks, bucket, presidio_config.as_ref()).await?;
    let (masked, failure, added) = collect_outputs(gathered);
    append_log_entries(request, added)?;
    if let Some(error) = failure {
        return Err(PresidioResponseError::from(error));
    }
    apply_masked_texts(messages, &tasks, &masked);
    Ok(tasks.len())
}

async fn run_checks(
    context: HookContext<'_>,
    tasks: &[TextTask],
    bucket: &'static str,
    config: Option<&PresidioPerRequestConfig>,
) -> Result<Vec<CheckOutcome>, PresidioResponseError> {
    // Each check owns an empty bucket of a few bytes, so the checks never copy request metadata between them.
    let base = empty_bucket(bucket)?;
    let mut checks = Vec::with_capacity(tasks.len());
    for task in tasks {
        let mut data = base.clone();
        let text = task.text.clone();
        checks.push(async move {
            let outcome = context
                .anonymizer
                .check_pii(
                    context.analyzer,
                    CheckPiiCall {
                        text: &text,
                        output_parse_pii: false,
                        presidio_config: config,
                        dynamic_params: context.dynamic_params,
                        pii_entities_config: context.pii_entities_config,
                        request_data: &mut data,
                    },
                )
                .await;
            (outcome, data)
        });
    }
    Ok(join_all(checks).await)
}

fn empty_bucket(bucket: &'static str) -> Result<RequestData, PresidioResponseError> {
    let mut data = RequestData::new();
    data.insert(bucket.to_owned(), to_raw(&RequestData::new())?);
    Ok(data)
}

fn collect_outputs(
    gathered: Vec<CheckOutcome>,
) -> (
    Vec<Option<String>>,
    Option<PresidioActionsError>,
    Vec<Box<RawValue>>,
) {
    let mut masked = Vec::with_capacity(gathered.len());
    let mut failure = None;
    let mut added = Vec::new();
    for (outcome, data) in gathered {
        added.extend(log_entries(&data));
        match outcome {
            Ok(text) => masked.push(Some(text)),
            Err(error) => {
                failure.get_or_insert(error);
                masked.push(None);
            }
        }
    }
    (masked, failure, added)
}
