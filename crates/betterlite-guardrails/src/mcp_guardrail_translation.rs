mod error;
mod input;
mod output;

pub mod mapping;

pub use error::McpGuardrailError;
pub use input::{
    Guardrail, GuardrailInput, GuardrailOutput, MAX_STRUCTURED_CONTENT_SCAN_DEPTH,
    McpGuardrailHandler,
};
pub use mapping::{CallType, guardrail_translation_mappings};

#[cfg(test)]
mod concurrent_conflict;
#[cfg(test)]
mod fallback_behavior;
#[cfg(test)]
mod input_writeback;
#[cfg(test)]
mod mapping_registration;
#[cfg(test)]
mod nesting_depth;
#[cfg(test)]
mod process_input;
#[cfg(test)]
mod process_output;
#[cfg(test)]
mod result_mutation;
#[cfg(test)]
mod text_count_mismatch;
