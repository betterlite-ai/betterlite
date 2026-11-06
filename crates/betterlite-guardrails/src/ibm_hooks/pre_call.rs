use super::IbmGuardrail;
use super::config::{EventHook, record_applied_guardrail, should_run_guardrail};
use super::data::{PromptData, iter_message_text};
use super::detection::check_contents;
use super::error::IbmHookError;

mod tests;

input_hook!(async_pre_call_hook, EventHook::PreCall);
