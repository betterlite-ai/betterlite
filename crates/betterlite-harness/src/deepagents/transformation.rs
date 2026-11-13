pub mod config;
pub mod gateway;
pub mod message;
pub mod tools;
pub mod update;

mod tests;

use crate::context::SessionContext;
use crate::errors::HarnessError;
use crate::types::Harness;

pub const INSTALL_HINT: &str =
    "Deep Agents is not installed. Run: pip install deepagents langchain-litellm";
pub const SKILLS_DIR: &str = ".deepagents/skills";
// graph supersteps per agent turn (model node, tools node, middleware hooks) for recursion_limit
pub const DEEPAGENTS_STEPS_PER_TURN: u32 = 6;
pub const BASE_RECURSION_LIMIT: u32 = 25;
pub const DEFAULT_RECURSION_LIMIT: u32 = 1000;
pub const MODEL_NODE: &str = "model";
// only these graph nodes produce new messages; middleware hooks may re-emit history
pub const EVENT_NODES: [&str; 2] = ["model", "tools"];

fn require_model<S>(context: &SessionContext<S>) -> Result<&str, HarnessError> {
    match context.settings.model.as_deref() {
        Some(model) if !model.is_empty() => Ok(model),
        Some(_) | None => Err(HarnessError::OptionsMismatch {
            given: "SessionSettings.model",
            harness: Harness::DeepAgents,
            expected: "a model name (Deep Agents needs model=)",
        }),
    }
}
