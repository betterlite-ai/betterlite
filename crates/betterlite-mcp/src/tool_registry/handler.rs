use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use betterlite_prompts::arguments::json::Object;
use thiserror::Error;

use crate::openapi_tools::output::OpenApiToolOutput;
use crate::openapi_tools::request_headers::OpenApiCallContext;
use crate::openapi_tools::tool::OpenApiToolError;

pub trait ToolHandler: Send + Sync {
    fn call<'call>(
        &'call self,
        arguments: &'call Object,
        context: &'call OpenApiCallContext,
    ) -> Pin<Box<dyn Future<Output = Result<OpenApiToolOutput, ToolHandlerError>> + Send + 'call>>;
}

#[derive(Debug, Error)]
pub enum ToolHandlerError {
    #[error(transparent)]
    OpenApi(#[from] OpenApiToolError),
    #[error("{0}")]
    Failed(String),
}

#[derive(Clone)]
pub struct LocalTool {
    pub name: String,
    pub description: String,
    pub input_schema: Object,
    pub handler: Arc<dyn ToolHandler>,
}
