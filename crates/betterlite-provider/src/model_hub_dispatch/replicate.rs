use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_logging::logging_pre_post::args::AdditionalArgs;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_models::decisions::Json;
use betterlite_prompts::builtin::Message;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;

use super::credentials::{replicate_base, replicate_key};
use super::json;
use super::{CallLogging, ChatContext, DispatchDeps, HubDispatchError};
use crate::replicate::poll::ReplicateCompletion;
use crate::replicate::poll::completion::{Replicate, ReplicateCall};
use crate::replicate::poll::{OutputPolicy, ReplicatePolling};
use crate::replicate::request::ReplicateDefaults;

pub type Completion = ReplicateCompletion;

// main.py L2811-L2816: a streamed call is logged after the module returns the wrapper, with no request data
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

// main.py L2773-L2824: the key and base chains, then the module's completion and the stream post_call it leaves to the caller
pub async fn complete(
    call: &mut ChatContext<'_>,
    deps: &DispatchDeps<'_>,
    log: &mut CallLogging<'_, '_>,
) -> Result<Completion, HubDispatchError> {
    let key = replicate_key(call.api_key, deps.globals, deps.environment);
    // L2795: LiteLLM resolves this base and ReplicateConfig.get_complete_url overwrites it, so it never reaches a request
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
