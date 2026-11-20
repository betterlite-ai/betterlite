mod gate;
pub mod plan;
mod tests;

use betterlite_context::agentic_loop_plan::AgenticLoopPlan;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::compression_interception::engine::CompressionEngine;
use crate::api::compression_interception::hook::CompressionInterception;

use self::plan::{RetrievalLoopError, RetrievalLoopRequest};

const CONTENT_FIELD: &str = "content";
const ID_FIELD: &str = "id";
const INPUT_FIELD: &str = "input";
const NAME_FIELD: &str = "name";
const TYPE_FIELD: &str = "type";
const TOOL_USE_TYPE: &str = "tool_use";
const TOOL_CALLS_FIELD: &str = "tool_calls";
const THINKING_BLOCKS_FIELD: &str = "thinking_blocks";
const TOOL_TYPE_FIELD: &str = "tool_type";
const COMPRESSION_RETRIEVAL_TYPE: &str = "compression_retrieval";

// The workspace denies multiple_inherent_impl and hook.rs holds the type's one inherent impl, so the callback's two operations live on this trait.
pub trait CompressionRetrievalLoop {
    #[must_use]
    fn should_run_agentic_loop(
        &self,
        response: &Json,
        tools: Option<&Json>,
    ) -> (bool, IndexMap<String, Json>);

    fn build_agentic_loop_plan(
        &self,
        tools: &IndexMap<String, Json>,
        request: &RetrievalLoopRequest<'_>,
    ) -> Result<AgenticLoopPlan, RetrievalLoopError>;
}

impl<Engine> CompressionRetrievalLoop for CompressionInterception<Engine>
where
    Engine: CompressionEngine,
{
    fn should_run_agentic_loop(
        &self,
        response: &Json,
        tools: Option<&Json>,
    ) -> (bool, IndexMap<String, Json>) {
        gate::should_run_agentic_loop(self, response, tools)
    }

    fn build_agentic_loop_plan(
        &self,
        tools: &IndexMap<String, Json>,
        request: &RetrievalLoopRequest<'_>,
    ) -> Result<AgenticLoopPlan, RetrievalLoopError> {
        plan::build_agentic_loop_plan(self, tools, request)
    }
}
