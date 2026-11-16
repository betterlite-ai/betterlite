use super::credentials::{replicate_base, replicate_key};
use super::json;
use super::{CallLogging, ChatContext, DispatchDeps, HubDispatchError};
use crate::replicate::poll::ReplicateCompletion;
use crate::replicate::poll::completion::{Replicate, ReplicateCall};
use crate::replicate::poll::{OutputPolicy, ReplicatePolling};
use crate::replicate::request::ReplicateDefaults;
use betterlite_logging::logging_pre_post::args::AdditionalArgs;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_models::decisions::Json;
use betterlite_prompts::builtin::Message;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use std::collections::BTreeMap;
use std::sync::Arc;
pub type Completion = ReplicateCompletion;
fn stream_post_call(
    log: &mut CallLogging<'_, '_>,
    messages: &[Message],
) -> Result<(), HubDispatchError> {
    post_call(
        log.state,
        &PostCall {
            original_response: Json::Null,
            input: json::convert(messages)?,
            additional_args: AdditionalArgs {
                api_base: None,
                headers: None,
                complete_input_dict: None,
                request_str: None,
                openai_sdk: None,
                passthrough: IndexMap::new(),
            },
        },
        log.env,
    );
    Ok(())
}
fn module<'deps, 'call>(
    deps: &'call DispatchDeps<'deps>,
    acompletion: bool,
    polling: ReplicatePolling,
) -> Replicate<'call>
where
    'deps: 'call,
{
    let services = &deps.replicate;
    Replicate {
        transport: Arc::clone(&services.transport),
        sleeper: Arc::clone(&services.sleeper),
        prompts: deps.prompts,
        counter: deps.replicate_counter,
        logger: services.logger,
        clock: deps.clock,
        polling,
        policy: if acompletion {
            OutputPolicy::Lenient
        } else {
            OutputPolicy::Strict
        },
    }
}
pub async fn complete(
    call: &mut ChatContext<'_>,
    deps: &DispatchDeps<'_>,
    log: &mut CallLogging<'_, '_>,
) -> Result<Completion, HubDispatchError> {
    let key = replicate_key(call.api_key, deps.globals, deps.environment);
    let _base = replicate_base(call.api_base, deps.globals, deps.environment);
    let empty = BTreeMap::new();
    let prompts = call
        .custom_prompt_dict
        .or(deps.globals.custom_prompt_dict)
        .unwrap_or(&empty);
    let stream = matches!(call.optional_params.get("stream"), Some(Json::Bool(true)));
    let polling = super::polling(deps.environment)?;
    let completion = module(deps, call.acompletion, polling)
        .complete(ReplicateCall {
            model: call.model,
            messages: call.messages,
            optional_params: call.optional_params,
            defaults: &ReplicateDefaults::default(),
            custom_prompts: prompts,
            api_key: key.as_ref().map(ExposeSecret::expose_secret),
            now: deps.now,
        })
        .await?;
    if stream {
        stream_post_call(log, call.messages)?;
    }
    Ok(completion)
}
