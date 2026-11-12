pub mod budget;
pub mod effort;
pub mod effort_param;
pub mod reasoning;
pub mod speed;
pub mod supported_params;
mod tests;

// LiteLLM's ANTHROPIC_MIN_THINKING_BUDGET_TOKENS, the floor a legacy minimal budget is raised to.
pub const MIN_THINKING_BUDGET_TOKENS: i64 = 1024;
