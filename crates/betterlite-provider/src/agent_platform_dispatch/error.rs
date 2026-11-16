use betterlite_secret_managers::secret_lookup::error::SecretLookupError;
use thiserror::Error;

use crate::a2a_chat::error::A2aFailure;
use crate::langflow_chat::error::LangFlowError;
use crate::langgraph_chat::error::LangGraphFailure;

use super::handler::HandlerError;

#[derive(Debug, Error)]
pub enum DispatchError {
    #[error(transparent)]
    A2a(#[from] A2aFailure),
    #[error(transparent)]
    LangGraph(#[from] LangGraphFailure),
    #[error(transparent)]
    LangFlow(#[from] LangFlowError),
    #[error("the {name} setting could not be read: {source}")]
    Environment {
        name: &'static str,
        #[source]
        source: SecretLookupError,
    },
    // LiteLLM's _complete_a2a raises this text as a plain Exception, before any error mapping
    #[error(
        "api_base is required for A2A provider. Either provide api_base parameter, set A2A_API_BASE environment variable, or register the agent in the proxy with model='a2a/<agent-name>'."
    )]
    MissingA2aApiBase,
    #[error(transparent)]
    Handler(#[from] HandlerError),
}
