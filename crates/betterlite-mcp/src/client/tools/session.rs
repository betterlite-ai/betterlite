use std::error::Error;
use std::future::Future;

use crate::tools::tool_calls::CallRequest;

use super::progress::{ProgressForwarder, ProgressHost};
use super::result::ToolCallResult;

pub trait ToolCallSession {
    type Error: Error + Send + Sync + 'static;

    fn call_tool<Host>(
        &mut self,
        request: &CallRequest,
        progress: &ProgressForwarder<'_, Host>,
        allow_input_required: bool,
    ) -> impl Future<Output = Result<ToolCallResult, Self::Error>> + Send
    where
        Host: ProgressHost + Sync;
}
