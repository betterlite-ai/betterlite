use betterlite_config::sections::gateway_provider_keys::Settings as SectionSettings;
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prompts::arguments::json::Object;
use betterlite_provider::gateway_provider_dispatch::chat::dispatch;
use betterlite_provider::gateway_provider_dispatch::context::{DispatchContext, DispatchSettings};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use serde_json::json;

use super::fixtures::{Recorder, environment, logging, section_settings};
use super::input::{Case, PreResolved, as_object, pre_resolved};
use super::input_failure;
use super::wire::{params_view, request_view};

const CHAT_ENDPOINT: &str = "/chat/completions";

// the case's request objects and section values, which the context borrows for one call
struct Prepared {
    settings: SectionSettings,
    optional_params: Object,
    class_config: Object,
    litellm_params: Object,
    pre: PreResolved,
}

pub(super) fn run(case: &Case) -> HelperResult {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?;
    let env = environment(case);
    let prepared = Prepared {
        settings: section_settings(case, &env),
        optional_params: as_object(case.optional_params.as_ref())?,
        class_config: as_object(case.class_config.as_ref())?,
        litellm_params: Object::new(),
        pre: pre_resolved(
            case.provider,
            case.api_base.as_ref(),
            case.api_key.as_ref(),
            &env,
        ),
    };
    let (mut state, prepost) = logging(&redactor)?;
    let mut log = LogContext {
        state: &mut state,
        env: &prepost,
    };
    let handler = Recorder::default();
    dispatch(
        case.provider.chat_tag(),
        &context(case, &prepared),
        &handler,
        &mut log,
    )
    .ok_or_else(|| input_failure(format!("{} has no chat arm", case.provider.slug())))?
    .map_err(|error| input_failure(format!("the chat arm failed: {error}")))?;
    let call = handler.first()?;
    let base = call
        .api_base
        .as_deref()
        .ok_or_else(|| input_failure(String::from("the chat arm resolved no api_base")))?;
    let url = format!("{base}{CHAT_ENDPOINT}");
    Ok(json!({
        "request": request_view(&url, call.api_key.as_ref(), call.headers.as_ref()),
        "params": params_view(&call.optional_params, call.stream, case.provider)?,
    }))
}

// the request context a chat arm reads, built from the case and the section values
fn context<'case>(case: &'case Case, prepared: &'case Prepared) -> DispatchContext<'case> {
    DispatchContext {
        acompletion: false,
        api_base: prepared.pre.api_base.as_deref(),
        api_key: prepared.pre.api_key.as_ref(),
        custom_llm_provider: case.provider.slug(),
        class_config: &prepared.class_config,
        headers: case.headers.as_ref(),
        litellm_params: &prepared.litellm_params,
        messages: &case.messages,
        model: &case.model,
        optional_params: &prepared.optional_params,
        stream: case.stream,
        timeout: None,
        settings: DispatchSettings::from(&prepared.settings),
    }
}
