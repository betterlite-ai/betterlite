use std::path::Path;
use std::sync::Arc;

use thiserror::Error;

use crate::tool_registry::handler::ToolHandler;

pub trait ToolHandlerLoader: Send + Sync {
    fn load(
        &self,
        handler: &str,
        config_file_path: Option<&Path>,
    ) -> Result<Option<Arc<dyn ToolHandler>>, HandlerLoadError>;
}

#[derive(Debug, Error)]
#[error("the handler {handler:?} could not be loaded")]
pub struct HandlerLoadError {
    pub handler: String,
}
