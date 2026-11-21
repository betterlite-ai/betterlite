use betterlite_error::rate_limit::InvalidRateLimitHeader;
use betterlite_error::rate_limit::ProxyRateLimitError;
use betterlite_error::rate_limit::ProxyRateLimitErrorInit;

use crate::atomic_counter::value::CounterValueError;

#[derive(Debug, thiserror::Error)]
pub enum SessionIterationLimitError {
    #[error("{0}")]
    Exceeded(Box<ProxyRateLimitError>),
    #[error("agent {agent_id} has a max_iterations that is not valid JSON")]
    IterationsJson {
        agent_id: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("agent {agent_id} has a max_iterations that is not an integer")]
    InvalidAgentIterations { agent_id: String },
    #[error("key metadata has a max_iterations that is not an integer")]
    InvalidKeyIterations,
    // int() on the local count raises in LiteLLM too, and the hook passes the error up
    #[error(transparent)]
    LocalCount(#[from] CounterValueError),
    // a rejected request whose error could not be built: no headers are sent today, so this stays unreachable
    #[error(transparent)]
    Rejection(#[from] InvalidRateLimitHeader),
}

impl SessionIterationLimitError {
    // ProxyRateLimitError.__init__ fails only over a header HTTP cannot carry, and this hook sends none
    pub(crate) fn rejected(init: ProxyRateLimitErrorInit) -> Self {
        match ProxyRateLimitError::new(init) {
            Ok(error) => Self::Exceeded(Box::new(error)),
            Err(source) => Self::Rejection(source),
        }
    }
}
