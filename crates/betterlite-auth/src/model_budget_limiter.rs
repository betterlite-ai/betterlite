pub mod cache;
pub mod charging;
pub mod clock;
pub mod enforcement;
pub mod error;
pub mod fallbacks;
mod tests;

use crate::budget_primitives::window_start::DurationParser;
use crate::model_budget_values::candidates::BedrockCatalog;
use cache::ModelBudgetCache;
use clock::BudgetClock;

pub struct ModelBudgetLimiter<C: ModelBudgetCache> {
    pub cache: C,
    pub durations: Box<dyn DurationParser + Send + Sync>,
    pub catalog: Box<dyn BedrockCatalog + Send + Sync>,
    pub clock: Box<dyn BudgetClock>,
    // litellm.budget_exceeded_status_code, 422 unless the caller supplies the configured value
    pub budget_exceeded_status: u16,
}
