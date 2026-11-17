use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use tokio::sync::mpsc::{Receiver, Sender};

use super::PresidioGuard;
use super::data::{store_request_data, to_request_data};
use crate::cato::error::ModelStreamError;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_selection::gate::GuardrailSelection as _;
use crate::presidio_actions::check::CheckPiiCall;
use crate::presidio_actions::error::PresidioActionsError;
use crate::presidio_actions::request_data::{
    PresidioPerRequestConfig, RequestData, presidio_settings_from_request,
};
use crate::presidio_response::error::PresidioResponseError;
use crate::presidio_response::messages::{
    TextTask, apply_masked_texts, collect_text_tasks, request_messages, store_messages,
};
use crate::presidio_response::wire::ResponseBody;
use crate::presidio_response::{HookContext, logging, post_call};
use crate::presidio_streams::assembly::StreamAssembly;
use crate::presidio_streams::{StreamEvent, StreamRun, hook};

pub(super) type Data = BTreeMap<String, GuardrailValue>;
type DynamicParams = BTreeMap<String, GuardrailValue>;

// The two async response hooks share their convert, call and write-back path, so one call shape selects them
pub(super) enum AsyncHook<'response, 'call> {
    PostCall(&'response mut ResponseBody),
    AsyncLogging {
        response: &'response mut ResponseBody,
        call_type: &'call str,
    },
}

fn prepare(
    guard: &PresidioGuard,
    data: &Data,
) -> Result<(RequestData, DynamicParams), GuardrailFailure> {
    let dynamic_params = guard.base.dynamic_request_body_params(data)?;
    Ok((to_request_data(data)?, dynamic_params))
}

const fn context<'context>(
    guard: &'context PresidioGuard,
    dynamic_params: &'context DynamicParams,
) -> HookContext<'context> {
    HookContext {
        analyzer: &guard.analyzer,
        anonymizer: &guard.anonymizer,
        pii_entities_config: &guard.pii_entities_config,
        dynamic_params,
        global_output_parse_pii: guard.global_output_parse_pii,
    }
}

pub(super) async fn pre_call(
    guard: &PresidioGuard,
    data: &mut Data,
    _call_type: &str,
) -> Result<(), GuardrailFailure> {
    if !guard
        .base
        .should_run_guardrail(data, GuardrailEventHooks::PreCall, &BTreeSet::new())?
    {
        return Ok(());
    }
    let dynamic_params = guard.base.dynamic_request_body_params(data)?;
    let mut request = to_request_data(data)?;
    let outcome = mask_request(guard, &mut request, &dynamic_params).await;
    // LiteLLM keeps the record a failed check appended, so the converted data goes back either way
    store_request_data(data, &request)?;
    outcome.map_err(GuardrailFailure::from)
}

pub(super) fn logging_hook(
    guard: &PresidioGuard,
    data: &mut Data,
    response: &mut ResponseBody,
    call_type: &str,
) -> Result<(), GuardrailFailure> {
    let (mut request, dynamic_params) = prepare(guard, data)?;
    logging::logging_hook(
        context(guard, &dynamic_params),
        &mut request,
        response,
        call_type,
    )?;
    store_request_data(data, &request)?;
    Ok(())
}

pub(super) async fn run_async(
    guard: &PresidioGuard,
    data: &mut Data,
    hook: AsyncHook<'_, '_>,
) -> Result<(), GuardrailFailure> {
    let (mut request, dynamic_params) = prepare(guard, data)?;
    let hook_context = context(guard, &dynamic_params);
    match hook {
        AsyncHook::PostCall(response) => {
            post_call::async_post_call_success_hook(hook_context, &mut request, response).await?;
        }
        AsyncHook::AsyncLogging {
            response,
            call_type,
        } => {
            logging::async_logging_hook(hook_context, &mut request, response, call_type).await?;
        }
    }
    store_request_data(data, &request)?;
    Ok(())
}

pub(super) async fn streaming_iterator<Assembly>(
    guard: &PresidioGuard,
    data: &mut Data,
    assembly: &Assembly,
    input: &mut Receiver<Result<StreamEvent, ModelStreamError>>,
    out: &Sender<StreamEvent>,
) -> Result<(), GuardrailFailure>
where
    Assembly: StreamAssembly,
{
    let (mut request, dynamic_params) = prepare(guard, data)?;
    let outcome = hook::streaming_iterator_hook(
        &mut StreamRun {
            context: context(guard, &dynamic_params),
            request_data: &mut request,
            assembly,
            out,
        },
        input,
    )
    .await;
    store_request_data(data, &request)?;
    outcome.map_err(GuardrailFailure::from)
}

async fn mask_request(
    guard: &PresidioGuard,
    request: &mut RequestData,
    dynamic_params: &DynamicParams,
) -> Result<(), PresidioResponseError> {
    let presidio_config = presidio_settings_from_request(request)?;
    let Some(mut messages) = request_messages(request)? else {
        return Ok(());
    };
    let tasks = collect_text_tasks(&messages);
    let masked = mask_texts(
        guard,
        &tasks,
        presidio_config.as_ref(),
        dynamic_params,
        request,
    )
    .await?;
    apply_masked_texts(&mut messages, &tasks, &masked);
    store_messages(request, &messages)
}

async fn mask_texts(
    guard: &PresidioGuard,
    tasks: &[TextTask],
    presidio_config: Option<&PresidioPerRequestConfig>,
    dynamic_params: &DynamicParams,
    request: &mut RequestData,
) -> Result<Vec<Option<String>>, PresidioActionsError> {
    let mut masked = Vec::with_capacity(tasks.len());
    let mut failure = None;
    for task in tasks {
        let outcome = guard
            .anonymizer
            .check_pii(
                &guard.analyzer,
                CheckPiiCall {
                    text: &task.text,
                    output_parse_pii: guard.analyzer.settings.output_parse_pii,
                    presidio_config,
                    dynamic_params,
                    pii_entities_config: &guard.pii_entities_config,
                    request_data: &mut *request,
                },
            )
            .await;
        match outcome {
            Ok(text) => masked.push(Some(text)),
            Err(error) => {
                failure.get_or_insert(error);
                masked.push(None);
            }
        }
    }
    if let Some(error) = failure {
        return Err(error);
    }
    Ok(masked)
}
