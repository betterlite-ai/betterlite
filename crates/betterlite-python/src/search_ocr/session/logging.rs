use std::time::Duration;

use betterlite_error::error::{ErrorClass, class_message};
use betterlite_logging::logging_pre_post::env::{DebugSettings, GlobalFlags, LogSink, PrePostEnv};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_sdk::call_wrapper::error::plain_error;
use chrono::Utc;
use indexmap::IndexMap;
use uuid::Uuid;

use super::SessionCore;
use crate::search_ocr::error::NativeFailure;

mod tests;

// litellm.search's logging object: the call's own ids, no messages and no callbacks to hand records on
pub(super) fn logging_state(
    core: &SessionCore,
    provider: &str,
) -> Result<LoggingState, NativeFailure> {
    let init = LoggingInit {
        model: format!("{provider}/search"),
        call_type: "search".to_owned(),
        start_time: Utc::now().fixed_offset(),
        litellm_call_id: Uuid::new_v4().to_string(),
        function_id: String::new(),
        messages: None,
        stream: Some(false),
        litellm_trace_id: None,
        kwargs: IndexMap::new(),
        litellm_params: IndexMap::new(),
        callback_dynamic_params: IndexMap::new(),
        trusted_callback_vars: Vec::new(),
        standard_built_in_tools_params: IndexMap::new(),
        dynamic_callbacks: DynamicCallbacks::default(),
        applied_guardrails: None,
        request_logging: RequestLoggingFlags::default(),
        callback_policy: CallbackPolicy::default(),
    };
    let mut context = CorrelationContext::default();
    let setup = CorrelationSetup {
        context: &mut context,
        settings: CorrelationSettings::default(),
    };
    LoggingState::new(
        init,
        setup,
        &core.redactor,
        || Uuid::new_v4().to_string(),
        || Duration::ZERO,
    )
    .map_err(|error| class_failure(plain_error(&error.to_string()), Vec::new()))
}

pub(super) fn pre_post_env<'env>(
    core: &'env SessionCore,
    sink: &'env dyn LogSink,
) -> PrePostEnv<'env> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: true,
            key_info: true,
        },
        redactor: &core.redactor,
        log: sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: Utc::now().fixed_offset(),
    }
}

// The provider _context names: custom_llm_provider, else the model's own prefix before the first slash
pub(super) fn provider_of<'model>(
    model: &'model str,
    custom_llm_provider: Option<&'model str>,
) -> &'model str {
    match custom_llm_provider {
        Some(provider) => provider,
        None => model.split_once('/').map_or("", |(prefix, _rest)| prefix),
    }
}

pub(super) fn class_failure(class: ErrorClass, headers: Vec<(String, String)>) -> NativeFailure {
    let message = class_message(&class);
    NativeFailure::Class {
        class,
        message,
        headers,
    }
}
