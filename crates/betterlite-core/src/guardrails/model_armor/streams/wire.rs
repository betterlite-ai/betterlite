use std::error::Error as StdError;
use std::future::Future;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::guardrails::streaming_rewrite::shapes::RequestData;

use super::assembly::Assembly;
use super::chunk::Chunk;
use super::errors::HttpError;
use super::surface::Surface;

// The parts of a Responses output item the scan reads: its texts and its tool calls, each call already encoded as JSON.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OutputParts {
    pub texts: Vec<String>,
    pub encoded_calls: Vec<String>,
}
// A Model Armor API failure, which fail_on_error decides about, and any other failure the wiring raises.
pub enum ScanFailure<Failure> {
    Api { detail: String },
    Unexpected(Failure),
}
// The guardrail machinery around Model Armor lives above this crate, so the buffered hook reaches it through this port.
pub trait StreamWire: Sync {
    type Failure: StdError + Send + Sync + 'static;
    fn is_anthropic_sse(&self, chunks: &[Chunk]) -> bool;
    fn is_sse_error(&self, chunks: &[Chunk]) -> bool;
    fn assemble_chat(&self, chunks: &[Chunk]) -> impl Future<Output = Option<Assembly>> + Send;
    fn assemble_anthropic(&self, chunks: &[Chunk])
    -> impl Future<Output = Option<Assembly>> + Send;
    fn model_text(&self, body: &GuardrailValue) -> Result<String, Self::Failure>;
    fn output_parts(&self, item: &GuardrailValue) -> OutputParts;
    fn scan(
        &self,
        content: &str,
    ) -> impl Future<Output = Result<GuardrailValue, ScanFailure<Self::Failure>>> + Send;
    fn should_block(&self, armor: &GuardrailValue, allow_sanitization: bool) -> bool;
    fn sanitized_content(&self, armor: &GuardrailValue) -> Option<String>;
    fn has_deidentify(&self, armor: &GuardrailValue) -> bool;
    fn logging_response(&self, armor: &GuardrailValue) -> GuardrailValue;
    fn block_detail(&self, message: &str, armor: &GuardrailValue) -> GuardrailValue;
    fn emit_sanitized(
        &self,
        surface: Surface,
        body: &GuardrailValue,
    ) -> Result<Vec<Chunk>, Self::Failure>;
    fn anthropic_error_frames(&self, message: &str) -> Result<Vec<Chunk>, Self::Failure>;
    fn responses_error_items(&self, error: &HttpError) -> Option<Vec<Chunk>>;
    fn applied_guardrail(&self, data: &mut RequestData, guardrail_name: &str);
}
