use std::future::Future;
use std::time::{Duration, Instant};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use super::RequestData;
use super::chunks::StreamChunk;
use super::errors::{BlockedChat, BlockedMessage, HookFailure};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanSource {
    Input,
    Output,
}

pub struct ScanCall<'call> {
    pub source: ScanSource,
    pub messages: Option<&'call [GuardrailValue]>,
    pub response: Option<&'call GuardrailValue>,
    pub request_data: &'call mut RequestData,
    pub event: GuardrailEventHooks,
}

pub struct RoleSelection {
    pub payload: Option<Vec<GuardrailValue>>,
    pub original: Option<Vec<GuardrailValue>>,
    pub target_indices: Option<Vec<usize>>,
}

pub struct UnifiedCall<'call> {
    pub request_data: &'call mut RequestData,
    pub chunks: Vec<StreamChunk>,
    pub buffer_until_moderated_default: bool,
}

// The helpers other tasks own, reached by name so this module never depends on the crates above it.
pub trait HookRuntime: Sync {
    // should_run_guardrail, U1
    fn should_run(&self, data: &RequestData, event: GuardrailEventHooks) -> bool;

    // get_guardrails_messages_for_call_type, U1
    fn messages_for_call_type(
        &self,
        call_type: &str,
        data: &RequestData,
    ) -> Option<Vec<GuardrailValue>>;

    // _prepare_guardrail_messages_for_role, B3721
    fn prepare_for_role(&self, messages: &[GuardrailValue]) -> RoleSelection;

    // _merge_filtered_messages, B3721
    fn merge_messages(
        &self,
        original: &[GuardrailValue],
        updated: &[GuardrailValue],
        indices: Option<&[usize]>,
    ) -> Vec<GuardrailValue>;

    // _update_messages_with_updated_bedrock_guardrail_response, B3721
    fn update_messages(
        &self,
        messages: &[GuardrailValue],
        scan: &GuardrailValue,
    ) -> Vec<GuardrailValue>;

    // _apply_masking_to_response, B3721
    fn mask_response(&self, response: &mut GuardrailValue, scan: &GuardrailValue);

    // add_guardrail_to_applied_guardrails_header, U3
    fn record_applied(&self, data: &mut RequestData);

    // make_bedrock_api_request, B3724
    fn scan(
        &self,
        call: ScanCall<'_>,
    ) -> impl Future<Output = Result<GuardrailValue, HookFailure>> + Send;

    // stream_chunk_builder and assemble_anthropic_sse_stream, B2227 and B3669
    fn assemble(
        &self,
        chunks: &[StreamChunk],
        raw_sse: bool,
    ) -> impl Future<Output = Option<GuardrailValue>> + Send;

    // anthropic_sse_chunks_from_response, B3669
    fn reassemble_sse(&self, response: &GuardrailValue) -> Result<Vec<Vec<u8>>, HookFailure>;

    // AnthropicMessagesHandler().build_block_sse_chunks, U1
    fn block_sse_chunks(&self, block: &BlockedMessage) -> Result<Vec<Vec<u8>>, HookFailure>;

    // MockResponseIterator over the replacement ModelResponse, U6
    fn block_chat_chunks(&self, block: &BlockedChat) -> Result<Vec<StreamChunk>, HookFailure>;

    // UnifiedLLMGuardrails().async_post_call_streaming_iterator_hook, U1
    fn delegate(
        &self,
        call: UnifiedCall<'_>,
    ) -> impl Future<Output = Result<Vec<StreamChunk>, HookFailure>> + Send;

    // time.monotonic(), whose elapsed the keepalive interval is read against
    fn elapsed(&self, since: Instant) -> Duration;

    // data.get("messages") for the post-call and streaming hooks, where no call type picks the list
    fn request_messages(&self, data: &RequestData) -> Option<Vec<GuardrailValue>> {
        let Some(GuardrailValue::Array(messages)) = data.get("messages") else {
            return None;
        };
        Some(messages.clone())
    }
}
