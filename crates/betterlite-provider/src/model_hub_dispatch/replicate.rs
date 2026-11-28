use super::credentials::{REPLICATE_KEY, Sources, resolve};
use super::json::log_opened_stream;
use super::{ChatContext, DispatchDeps, HubDispatchError};
use crate::replicate::poll::ReplicateCompletion;
use crate::replicate::poll::completion::{Replicate, ReplicateCall};
use crate::replicate::poll::{OutputPolicy, ReplicatePolling};
use crate::replicate::request::ReplicateDefaults;
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use secrecy::ExposeSecret;
use std::collections::BTreeMap;
use std::sync::Arc;
pub type Completion = ReplicateCompletion;
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
pub async fn complete<'call>(
    call: &'call mut ChatContext<'_>,
    deps: &'call DispatchDeps<'_>,
    log: &'call mut LogContext<'_, '_>,
) -> Result<Completion, HubDispatchError> {
    let sources = Sources {
        call: call.api_key.map(ExposeSecret::expose_secret),
        globals: deps.globals,
        env: deps.environment,
    };
    let key = resolve(REPLICATE_KEY, &sources, &mut Object::new())?;
    let empty = BTreeMap::new();
    let prompts = call
        .custom_prompt_dict
        .or(deps.globals.custom_prompt_dict)
        .unwrap_or(&empty);
    let stream = matches!(
        call.optional_params.get("stream"),
        Some(PromptJson::Bool(true))
    );
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
        log_opened_stream(log, call.messages, None)?;
    }
    Ok(completion)
}
