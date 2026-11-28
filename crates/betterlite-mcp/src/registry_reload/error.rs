use betterlite_db::mcp::table_access::types::McpTableError;
use thiserror::Error;

use crate::config_loader::error::ConfigError;
use crate::server_build::BuildError;

#[derive(Debug, Error)]
pub enum MutationError {
    #[error(transparent)]
    Build(Box<BuildError>),
    #[error(transparent)]
    Register(Box<ConfigError>),
    #[error(transparent)]
    Rows(Box<McpTableError>),
}

impl From<BuildError> for MutationError {
    fn from(source: BuildError) -> Self {
        Self::Build(Box::new(source))
    }
}

impl From<ConfigError> for MutationError {
    fn from(source: ConfigError) -> Self {
        Self::Register(Box::new(source))
    }
}

impl From<McpTableError> for MutationError {
    fn from(source: McpTableError) -> Self {
        Self::Rows(Box::new(source))
    }
}
