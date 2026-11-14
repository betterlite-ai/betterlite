pub mod error;
pub mod inspect;
pub mod round;
pub mod run;
pub mod tail;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, VecDeque};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::routes::CallType;

// What one step of the loop writes: a stream item, or the byte frames of a block message
#[derive(Debug, Clone, PartialEq)]
pub enum StreamOutput {
    Item(GuardrailValue),
    BlockChunks(Vec<Vec<u8>>),
}

// What one run of the loop needs besides its source stream
pub struct StreamLoopParams<'run, T, G> {
    pub translation: &'run T,
    pub guardrail: &'run G,
    pub request_data: &'run mut BTreeMap<String, GuardrailValue>,
    pub user_api_key_dict: &'run BTreeMap<String, GuardrailValue>,
    pub call_type: CallType,
    pub sampling_rate: usize,
    pub end_of_stream_only: bool,
}

// The three lists a round reads or writes, bundled to keep a round under the argument limit
pub(crate) struct StreamBuffers<'buffers> {
    pub(crate) responses_so_far: &'buffers mut Vec<GuardrailValue>,
    pub(crate) responses_yielded: &'buffers mut Vec<GuardrailValue>,
    pub(crate) outputs: &'buffers mut VecDeque<StreamOutput>,
}
