use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use betterlite_prompts::arguments::json::Object;

use crate::openapi_tools::output::OpenApiToolOutput;
use crate::openapi_tools::request_headers::OpenApiCallContext;
use crate::openapi_tools::tool::OpenApiTool;
use crate::openapi_tools::transport::OpenApiTransport;
use crate::tool_registry::handler::{ToolHandler, ToolHandlerError};

pub struct OpenApiToolHandler {
    pub tool: OpenApiTool,
    pub transport: Arc<dyn OpenApiTransport>,
}

impl ToolHandler for OpenApiToolHandler {
    fn call<'call>(
        &'call self,
        arguments: &'call Object,
        context: &'call OpenApiCallContext,
    ) -> Pin<Box<dyn Future<Output = Result<OpenApiToolOutput, ToolHandlerError>> + Send + 'call>>
    {
        Box::pin(async move {
            self.tool
                .call(arguments, context, self.transport.as_ref())
                .await
                .map_err(ToolHandlerError::from)
        })
    }
}
