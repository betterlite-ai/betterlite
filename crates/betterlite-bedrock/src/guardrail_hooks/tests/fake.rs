use std::future::{Future, ready};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use crate::guardrail_hooks::RequestData;
use crate::guardrail_hooks::chunks::StreamChunk;
use crate::guardrail_hooks::errors::{BlockedChat, BlockedMessage, HookFailure};
use crate::guardrail_hooks::runtime::{
    HookRuntime, RoleSelection, ScanCall, ScanSource, UnifiedCall,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanRecord {
    pub source: ScanSource,
    pub message_count: usize,
    pub has_response: bool,
    pub event: GuardrailEventHooks,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DelegateRecord {
    pub buffer_until_moderated_default: bool,
    pub chunk_count: usize,
}

pub struct FakeRuntime {
    pub should_run: bool,
    pub messages: Option<Vec<GuardrailValue>>,
    pub selection: RoleSelection,
    pub scan_result: Result<GuardrailValue, HookFailure>,
    pub updated: Option<Vec<GuardrailValue>>,
    pub masked_response: Option<GuardrailValue>,
    pub assembled: Option<GuardrailValue>,
    pub sse_frames: Vec<Vec<u8>>,
    pub block_frames: Vec<Vec<u8>>,
    pub replacement_chunks: Vec<StreamChunk>,
    pub elapsed: Duration,
    pub scans: Mutex<Vec<ScanRecord>>,
    pub delegations: Mutex<Vec<DelegateRecord>>,
    pub masked: Mutex<Vec<GuardrailValue>>,
    pub refused: Mutex<usize>,
    pub applied: Mutex<usize>,
    pub blocked_sse: Mutex<Vec<BlockedMessage>>,
    pub blocked_chat: Mutex<Vec<BlockedChat>>,
}

pub fn base() -> FakeRuntime {
    FakeRuntime {
        should_run: true,
        messages: None,
        selection: RoleSelection {
            payload: None,
            original: None,
            target_indices: None,
        },
        scan_result: Err(HookFailure::Failed {
            message: "no scan scripted".to_owned(),
        }),
        updated: None,
        masked_response: None,
        assembled: None,
        sse_frames: Vec::new(),
        block_frames: Vec::new(),
        replacement_chunks: Vec::new(),
        elapsed: Duration::ZERO,
        scans: Mutex::new(Vec::new()),
        delegations: Mutex::new(Vec::new()),
        masked: Mutex::new(Vec::new()),
        refused: Mutex::new(0),
        applied: Mutex::new(0),
        blocked_sse: Mutex::new(Vec::new()),
        blocked_chat: Mutex::new(Vec::new()),
    }
}

impl HookRuntime for FakeRuntime {
    fn should_run(&self, _data: &RequestData, _event: GuardrailEventHooks) -> bool {
        self.should_run
    }

    fn messages_for_call_type(
        &self,
        _call_type: &str,
        _data: &RequestData,
    ) -> Option<Vec<GuardrailValue>> {
        self.messages.clone()
    }

    fn prepare_for_role(&self, _messages: &[GuardrailValue]) -> RoleSelection {
        RoleSelection {
            payload: self.selection.payload.clone(),
            original: self.selection.original.clone(),
            target_indices: self.selection.target_indices.clone(),
        }
    }

    fn merge_messages(
        &self,
        original: &[GuardrailValue],
        updated: &[GuardrailValue],
        indices: Option<&[usize]>,
    ) -> Vec<GuardrailValue> {
        let Some(targets) = indices.filter(|items| !items.is_empty()) else {
            return updated.to_vec();
        };
        let mut merged = if original.is_empty() {
            updated.to_vec()
        } else {
            original.to_vec()
        };
        for (index, message) in targets.iter().zip(updated) {
            if let Some(slot) = merged.get_mut(*index) {
                slot.clone_from(message);
            }
        }
        merged
    }

    fn update_messages(
        &self,
        messages: &[GuardrailValue],
        _scan: &GuardrailValue,
    ) -> Vec<GuardrailValue> {
        match &self.updated {
            Some(updated) => updated.clone(),
            None => messages.to_vec(),
        }
    }

    fn mask_response(&self, response: &mut GuardrailValue, _scan: &GuardrailValue) {
        self.masked
            .lock()
            .expect("mask records")
            .push(response.clone());
        if let Some(masked) = &self.masked_response {
            response.clone_from(masked);
        }
    }

    fn record_applied(&self, data: &mut RequestData) {
        let mut applied = self.applied.lock().expect("applied count");
        *applied = applied.saturating_add(1);
        data.insert("applied".to_owned(), GuardrailValue::Boolean(true));
    }

    fn scan(
        &self,
        call: ScanCall<'_>,
    ) -> impl Future<Output = Result<GuardrailValue, HookFailure>> + Send {
        let record = ScanRecord {
            source: call.source,
            message_count: call.messages.map_or(0, <[GuardrailValue]>::len),
            has_response: call.response.is_some(),
            event: call.event,
        };
        self.scans.lock().expect("scan records").push(record);
        ready(self.scan_result.clone())
    }

    fn assemble(
        &self,
        _chunks: &[StreamChunk],
        _raw_sse: bool,
    ) -> impl Future<Output = Option<GuardrailValue>> + Send {
        ready(self.assembled.clone())
    }

    fn reassemble_sse(&self, _response: &GuardrailValue) -> Result<Vec<Vec<u8>>, HookFailure> {
        Ok(self.sse_frames.clone())
    }

    fn block_sse_chunks(&self, block: &BlockedMessage) -> Result<Vec<Vec<u8>>, HookFailure> {
        self.blocked_sse
            .lock()
            .expect("sse block records")
            .push(block.clone());
        Ok(self.block_frames.clone())
    }

    fn block_chat_chunks(&self, block: &BlockedChat) -> Result<Vec<StreamChunk>, HookFailure> {
        self.blocked_chat
            .lock()
            .expect("chat block records")
            .push(block.clone());
        Ok(self.replacement_chunks.clone())
    }

    fn delegate(
        &self,
        call: UnifiedCall<'_>,
    ) -> impl Future<Output = Result<Vec<StreamChunk>, HookFailure>> + Send {
        self.note_delegation(DelegateRecord {
            buffer_until_moderated_default: call.buffer_until_moderated_default,
            chunk_count: call.chunks.len(),
        });
        ready(Ok(call.chunks))
    }

    fn note_delegation(&self, record: DelegateRecord) {
        self.delegations
            .lock()
            .expect("delegation records")
            .push(record);
    }

    fn elapsed(&self, _since: Instant) -> Duration {
        self.elapsed
    }
}
