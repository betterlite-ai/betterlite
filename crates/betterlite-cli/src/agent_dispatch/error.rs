use std::io;

use thiserror::Error;

use crate::agent_launch::AgentRunError as LaunchError;
use crate::agent_run::error::AgentRunError;
use crate::login::error::LoginError;

#[derive(Debug, Error)]
pub enum AgentDispatchError {
    #[error(
        "No LiteLLM key found. Set LITELLM_PROXY_API_KEY (or pass --api-key) for non-interactive use, or run `lite login` from a terminal."
    )]
    NoKey,
    #[error("Login did not produce an API key; cannot start the agent.")]
    LoginProducedNoKey,
    #[error(transparent)]
    Login(#[from] LoginError),
    #[error(transparent)]
    Agent(#[from] AgentRunError),
    #[error(transparent)]
    Launch(#[from] LaunchError),
    #[error("could not write to the terminal: {0}")]
    Write(#[from] io::Error),
    #[error("the async runtime could not start: {0}")]
    Runtime(#[source] io::Error),
}
