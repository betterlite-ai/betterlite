use betterlite_context::agentic_loop_plan::AgenticLoopPlan;

use crate::sandbox_lifecycle::cache_operations::Clock;
use crate::sandbox_lifecycle::lifecycle::{SandboxConfig, SandboxRegistry};
use crate::sandbox_lifecycle::resolution::SandboxToolRegistry;

use super::chat_plan::build_chat_plan;
use super::error::PlanError;
use super::gate::is_chat_completion_surface;
use super::initialization::InterceptionLogger;
use super::request::PlanRequest;
use super::responses_plan::build_responses_plan;

// LiteLLM's async_build_agentic_loop_plan: the chat surface takes the chat builder, every other surface the Responses one
pub async fn build_agentic_loop_plan<C, R, T, Clk>(
    logger: &mut InterceptionLogger<C>,
    registry: &R,
    tool_registry: &T,
    clock: &Clk,
    request: &PlanRequest<'_>,
) -> Result<AgenticLoopPlan, PlanError<C::Error>>
where
    C: SandboxConfig,
    R: SandboxRegistry<Error = C::Error>,
    T: SandboxToolRegistry,
    Clk: Clock,
{
    if is_chat_completion_surface(request.kwargs) {
        build_chat_plan(logger, registry, tool_registry, clock, request).await
    } else {
        build_responses_plan(logger, registry, tool_registry, clock, request).await
    }
}

include!("dispatch/tests/cases.rs");
