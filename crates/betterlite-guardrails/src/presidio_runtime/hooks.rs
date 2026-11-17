use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::PresidioGuard;
use crate::presidio_actions::check::CheckPiiCall;
use crate::presidio_actions::error::PresidioActionsError;
use crate::presidio_actions::request_data::{
    PresidioPerRequestConfig, RequestData, presidio_settings_from_request,
};
use crate::presidio_response::HookContext;
use crate::presidio_response::error::PresidioResponseError;
use crate::presidio_response::messages::{
    TextTask, apply_masked_texts, collect_text_tasks, request_messages, store_messages,
};

pub(super) type Data = BTreeMap<String, GuardrailValue>;
pub(super) type DynamicParams = BTreeMap<String, GuardrailValue>;

pub(super) const fn context<'context>(
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

pub(super) async fn mask_request(
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
