use thiserror::Error;

use crate::api::messages_thinking::session::SessionKeyError;

#[derive(Debug, Error)]
pub enum OptionsError {
    #[error(transparent)]
    SessionKey(#[from] SessionKeyError),
    #[error("thinking.budget_tokens must be an integer")]
    BudgetNotInteger,
    #[error("output_config.effort must be a string")]
    EffortNotText,
}
