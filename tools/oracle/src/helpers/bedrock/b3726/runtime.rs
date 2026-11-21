use std::collections::BTreeSet;
use std::future::{Future, ready};
use std::time::{Duration, Instant};

use betterlite_bedrock::guardrail_hooks::RequestData;
use betterlite_bedrock::guardrail_hooks::StreamingSettings;
use betterlite_bedrock::guardrail_hooks::chunks::StreamChunk;
use betterlite_bedrock::guardrail_hooks::errors::{BlockedChat, BlockedMessage, HookFailure};
use betterlite_bedrock::guardrail_hooks::runtime::{
    HookRuntime, RoleSelection, ScanCall, ScanSource, UnifiedCall,
};
use betterlite_bedrock_guardrails::bedrock_transport::BedrockGuardrail;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_core::api::messages_chat_output::records::ResponseRecord;
use betterlite_guardrails::anthropic_sse::sse_chunks_from_response::sse_chunks_from_response;
use betterlite_guardrails::bedrock_apply::{ApiInvocation, make_bedrock_api_request};
use betterlite_guardrails::bedrock_checks::request::ChecksScan;
use betterlite_guardrails::bedrock_masking::{
    Masking, apply_to_response, extract_masked_texts, update_messages as masking_update,
};
use betterlite_guardrails::bedrock_message_selection::prepare_for_role as prepare_wire;
use betterlite_guardrails::bedrock_message_selection::reconcile;
use betterlite_guardrails::bedrock_message_selection::wire::ModelResponse;
use betterlite_guardrails::bedrock_records::{Response as GuardrailResponse, Source};
use betterlite_guardrails::guardrail_base::GuardrailBase;
use betterlite_guardrails::guardrail_message_scope::blocked_usage::OriginalResponse;
use betterlite_guardrails::guardrail_selection::gate::GuardrailSelection as _;
use betterlite_guardrails::tool_permission_requests::header_mutation::record_applied_guardrail;
use betterlite_models::catalog::BundledCatalog;
use betterlite_server::guardrails::anthropic_translation::block_continuation::{
    BlockedException, build_block_sse_chunks,
};

use super::assembly::{assemble, block_chat_chunks};
use super::replies::Sent;
use super::support::{Creds, ENVIRONMENT, helper_now};
use super::values::{failed, hook_failure, to_wire, via_json};

const BLOCK_MESSAGE_ID: &str = "msg_block";

// The production hooks run against this stand-in for the proxy: the guardrail, its base settings and a scripted transport.
pub(super) struct OracleRuntime {
    guardrail: BedrockGuardrail,
    base: GuardrailBase,
    pub(super) settings: StreamingSettings,
    pub(super) transport: Sent,
    elapsed: Duration,
}

impl OracleRuntime {
    pub(super) fn new(
        guardrail: BedrockGuardrail,
        base: GuardrailBase,
        settings: StreamingSettings,
        transport: Sent,
        elapsed: Duration,
    ) -> Self {
        Self {
            guardrail,
            base,
            settings,
            transport,
            elapsed,
        }
    }
}

impl HookRuntime for OracleRuntime {
    fn should_run(&self, data: &RequestData, event: GuardrailEventHooks) -> bool {
        self.base
            .should_run_guardrail(data, event, &BTreeSet::new())
            .unwrap_or(false)
    }

    // The tested call types read data["messages"] directly; the tool-cache branches belong to U1.
    fn messages_for_call_type(
        &self,
        _call_type: &str,
        data: &RequestData,
    ) -> Option<Vec<GuardrailValue>> {
        self.request_messages(data)
    }

    fn prepare_for_role(&self, messages: &[GuardrailValue]) -> RoleSelection {
        let Ok(wire) = to_wire(messages) else {
            return RoleSelection {
                payload: None,
                original: None,
                target_indices: None,
            };
        };
        let result = prepare_wire(Some(&wire), self.guardrail.latest_role_message_only);
        RoleSelection {
            payload: result.payload_messages.as_deref().and_then(via_json),
            original: result.original_messages.as_deref().and_then(via_json),
            target_indices: result.target_indices,
        }
    }

    fn merge_messages(
        &self,
        original: &[GuardrailValue],
        updated: &[GuardrailValue],
        indices: Option<&[usize]>,
    ) -> Vec<GuardrailValue> {
        let (Ok(originals), Ok(replacements)) = (to_wire(original), to_wire(updated)) else {
            return updated.to_vec();
        };
        let merged = reconcile::messages(Some(&originals), &replacements, indices);
        via_json(&merged).unwrap_or_else(|| updated.to_vec())
    }

    fn update_messages(
        &self,
        messages: &[GuardrailValue],
        scan: &GuardrailValue,
    ) -> Vec<GuardrailValue> {
        let (Ok(items), Some(scan_response)) = (
            to_wire(messages),
            via_json::<GuardrailValue, GuardrailResponse>(scan),
        ) else {
            return messages.to_vec();
        };
        let flags = Masking {
            request: false,
            response: false,
        };
        via_json(&masking_update(&items, &scan_response, &flags))
            .unwrap_or_else(|| messages.to_vec())
    }

    fn mask_response(&self, response: &mut GuardrailValue, scan: &GuardrailValue) {
        let (Some(mut model), Some(scan_response)) = (
            via_json::<GuardrailValue, ModelResponse>(response),
            via_json::<GuardrailValue, GuardrailResponse>(scan),
        ) else {
            return;
        };
        apply_to_response(&mut model, &extract_masked_texts(Some(&scan_response)));
        if let Some(updated) = via_json(&model) {
            *response = updated;
        }
    }

    fn record_applied(&self, data: &mut RequestData) {
        let name = self
            .base
            .settings
            .guardrail_name
            .as_deref()
            .unwrap_or_default();
        record_applied_guardrail(data, name);
    }

    fn scan(
        &self,
        call: ScanCall<'_>,
    ) -> impl Future<Output = Result<GuardrailValue, HookFailure>> + Send {
        let messages = call.messages.map(to_wire);
        let response = call.response.map(|value| {
            via_json::<GuardrailValue, ModelResponse>(value)
                .ok_or_else(|| failed("the guardrail response could not be decoded"))
        });
        let source = match call.source {
            ScanSource::Input => Source::Input,
            ScanSource::Output => Source::Output,
        };
        let event = call.event;
        let request_data = call.request_data;
        async move {
            let messages = messages.transpose()?;
            let response = response.transpose()?;
            let catalog = BundledCatalog::default().get();
            let mut invocation = ApiInvocation {
                guardrail: &self.guardrail,
                base: &self.base,
                request_data,
                log_target: None,
                environment: ENVIRONMENT,
                catalog: catalog.as_ref(),
                now: helper_now,
            };
            let scan = ChecksScan {
                source,
                messages: messages.as_deref(),
                response: response.as_ref(),
                logging_event_type: Some(event),
            };
            match make_bedrock_api_request(&mut invocation, scan, &Creds, &self.transport).await {
                Ok(result) => via_json(&result)
                    .ok_or_else(|| failed("the guardrail response could not be encoded")),
                Err(failure) => Err(hook_failure(failure)),
            }
        }
    }

    fn assemble(
        &self,
        chunks: &[StreamChunk],
        raw_sse: bool,
    ) -> impl Future<Output = Option<GuardrailValue>> + Send {
        ready(assemble(chunks, raw_sse))
    }

    fn reassemble_sse(&self, response: &GuardrailValue) -> Result<Vec<Vec<u8>>, HookFailure> {
        let record: ResponseRecord =
            via_json(response).ok_or_else(|| failed("the masked response could not be encoded"))?;
        sse_chunks_from_response(&record).map_err(|error| HookFailure::Failed {
            message: error.to_string(),
        })
    }

    fn block_sse_chunks(&self, block: &BlockedMessage) -> Result<Vec<Vec<u8>>, HookFailure> {
        let original = match &block.original_response {
            GuardrailValue::Object(fields) => OriginalResponse::Response(fields.clone()),
            GuardrailValue::Null
            | GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Text(_)
            | GuardrailValue::Array(_) => OriginalResponse::Absent,
        };
        let blocked = BlockedException {
            message: &block.message,
            model: &block.model,
            message_id: BLOCK_MESSAGE_ID,
            original_response: &original,
        };
        build_block_sse_chunks(&blocked, false, None).map_err(|error| HookFailure::Failed {
            message: error.to_string(),
        })
    }

    fn block_chat_chunks(&self, block: &BlockedChat) -> Result<Vec<StreamChunk>, HookFailure> {
        block_chat_chunks(block)
    }

    fn delegate(
        &self,
        call: UnifiedCall<'_>,
    ) -> impl Future<Output = Result<Vec<StreamChunk>, HookFailure>> + Send {
        ready(Ok(call.chunks))
    }

    fn elapsed(&self, _since: Instant) -> Duration {
        self.elapsed
    }
}
