use betterlite_guardrails::ibm_hooks::IbmGuardrail;
use betterlite_guardrails::ibm_hooks::config::should_run_guardrail;
use betterlite_guardrails::ibm_hooks::data::{PromptData, iter_message_text};
use betterlite_guardrails::ibm_hooks::during_call::async_moderation_hook;
use betterlite_guardrails::ibm_hooks::error::IbmHookError;
use betterlite_guardrails::ibm_hooks::post_call::async_post_call_success_hook;
use betterlite_guardrails::ibm_hooks::pre_call::async_pre_call_hook;
use betterlite_guardrails::ibm_hooks::response::ModelResponse;
use betterlite_guardrails::ibm_hooks::streaming::async_streaming_iterator_hook;
use futures_util::future::join;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};
use tokio::sync::mpsc;

use super::config::{failure, guardrail};
use super::{HookInput, PostCallInput, ShouldRunInput, StreamingInput};
use crate::case::UpstreamAnswer;
use crate::helpers::{HelperError, HelperResult};
use crate::mock::MockProvider;
use crate::routing_mock_scripts::{MockScript, ProviderCall};

#[derive(Debug, Clone, Copy)]
pub(super) enum InputEvent {
    PreCall,
    DuringCall,
}

pub(super) fn input_hook(args: &HookInput, event: InputEvent) -> HelperResult {
    let mut data: PromptData = serde_json::from_value(args.data.clone())
        .map_err(|error| failure("ValueError", error.to_string()))?;
    let calls = iter_message_text(&data).len().max(1);
    let mock = mock_for(&args.answer, calls)?;
    let guardrail = guardrail(&args.settings, Some(&mock.base_url))?;
    let runtime = runtime()?;
    let outcome = runtime.block_on(run_input_hook(&guardrail, &mut data, event));
    let recorded = finish(mock)?;
    Ok(hook_output(&recorded, outcome.is_err(), &data))
}

async fn run_input_hook(
    guardrail: &IbmGuardrail,
    data: &mut PromptData,
    event: InputEvent,
) -> Result<(), IbmHookError> {
    match event {
        InputEvent::PreCall => async_pre_call_hook(guardrail, data).await,
        InputEvent::DuringCall => async_moderation_hook(guardrail, data).await,
    }
}

pub(super) fn post_call(args: &PostCallInput) -> HelperResult {
    let mut data: PromptData = serde_json::from_value(args.data.clone())
        .map_err(|error| failure("ValueError", error.to_string()))?;
    let response: ModelResponse = serde_json::from_value(args.response.clone())
        .map_err(|error| failure("ValueError", error.to_string()))?;
    let calls = response
        .choices
        .iter()
        .filter_map(|choice| choice.message.content.as_deref())
        .filter(|text| !text.is_empty())
        .count()
        .max(1);
    let mock = mock_for(&args.answer, calls)?;
    let guardrail = guardrail(&args.settings, Some(&mock.base_url))?;
    let runtime = runtime()?;
    let outcome = runtime.block_on(async_post_call_success_hook(
        &guardrail, &mut data, &response,
    ));
    let recorded = finish(mock)?;
    Ok(hook_output(&recorded, outcome.is_err(), &data))
}

pub(super) fn streaming(args: &StreamingInput) -> HelperResult {
    let runtime = runtime()?;
    let items = runtime.block_on(relay_items(&args.items));
    Ok(json!({"items": items}))
}

async fn relay_items(items: &[Value]) -> Vec<Value> {
    let capacity = items.len().max(1);
    let (sender, response) = mpsc::channel(capacity);
    let (output, mut relayed) = mpsc::channel(capacity);
    let hook = async_streaming_iterator_hook(response, output);
    let feed = async move {
        for item in items {
            if sender.send(item.clone()).await.is_err() {
                break;
            }
        }
    };
    let ((), ()) = join(hook, feed).await;
    let mut collected = Vec::new();
    while let Some(item) = relayed.recv().await {
        collected.push(item);
    }
    collected
}

pub(super) fn should_run(args: &ShouldRunInput) -> HelperResult {
    let guardrail = guardrail(&args.settings, None)?;
    Ok(json!({
        "should_run": should_run_guardrail(&guardrail, args.event_type),
    }))
}

fn mock_for(answer: &UpstreamAnswer, calls: usize) -> Result<MockProvider, Box<HelperError>> {
    let answers = vec![answer.clone(); calls];
    let script = MockScript::try_from(answers.as_slice())
        .map_err(|error| failure("MockError", error.to_string()))?;
    MockProvider::start(script).map_err(|error| failure("MockError", error.to_string()))
}

fn finish(mock: MockProvider) -> Result<Vec<ProviderCall>, Box<HelperError>> {
    mock.finish()
        .map_err(|error| failure("MockError", error.to_string()))
}

fn hook_output(recorded: &[ProviderCall], raised: bool, data: &PromptData) -> Value {
    json!({
        "requests": requests(recorded),
        "raised": raised,
        "metadata": data.metadata,
        "litellm_metadata": data.litellm_metadata,
    })
}

fn requests(recorded: &[ProviderCall]) -> Vec<Value> {
    recorded
        .iter()
        .map(|call| json!({"path": call.request.path, "body": call.request.body}))
        .collect()
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failure("OSError", error.to_string()))
}
