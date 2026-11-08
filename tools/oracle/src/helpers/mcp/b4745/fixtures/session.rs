use std::collections::VecDeque;
use std::error::Error;
use std::future::{Future, pending, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_mcp::client::sessions::error::SessionError;
use betterlite_mcp::client::sessions::negotiate::{
    ClientSession, InitializeOutcome, InitializeRequest,
};
use betterlite_mcp::client::tools::progress::{ProgressForwarder, ProgressHost};
use betterlite_mcp::client::tools::result::{InputRequiredResult, ToolCallResult};
use betterlite_mcp::client::tools::session::ToolCallSession;
use betterlite_mcp::tools::pagination::{McpSession, Page};
use betterlite_mcp::tools::tool_calls::CallRequest;
use tokio::time::sleep;

use super::super::input::{Frame, InputRequirement};
use super::types::{Script, Simulated, complete, page_of};

pub(crate) struct Init;

impl InitializeOutcome for Init {
    fn protocol_version(&self) -> &'static str {
        "2025-06-18"
    }

    fn instructions(&self) -> Option<&str> {
        None
    }
}

pub(crate) struct Session<Source> {
    pub(crate) script: Arc<Script<Source>>,
    pub(crate) frames: Mutex<VecDeque<Frame>>,
}

impl<Source> ClientSession for Session<Source>
where
    Source: Error + Send + Sync + 'static,
{
    type Error = Source;
    type InitializeResult = Init;

    fn initialize(&mut self) -> impl Future<Output = Result<Init, Source>> + Send {
        ready(Ok(Init))
    }

    fn send_initialize(
        &mut self,
        _request: InitializeRequest,
    ) -> impl Future<Output = Result<Init, Source>> + Send {
        ready(Ok(Init))
    }

    fn adopt(&mut self, _result: &Init) {}

    fn send_initialized(&mut self) -> impl Future<Output = Result<(), Source>> + Send {
        ready(Ok(()))
    }
}

impl<Source> McpSession for Session<Source>
where
    Source: Simulated,
{
    type Error = SessionError<Source>;
    type Output = ToolCallResult;

    async fn list_tools(&mut self, _cursor: Option<&str>) -> Result<Page, Self::Error> {
        if self.script.cancel {
            self.script.cancelled.cancel();
            pending::<()>().await;
        }
        if let Some(error) = self.script.list_error.clone() {
            return Err(SessionError::Runtime(Arc::new(error)));
        }
        let popped = self
            .frames
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        let Some(frame) = popped else {
            return Ok(Page::default());
        };
        sleep(Duration::from_millis(frame.delay_ms)).await;
        Ok(page_of(&frame))
    }

    fn call_tool(
        &mut self,
        _request: CallRequest,
    ) -> impl Future<Output = Result<ToolCallResult, SessionError<Source>>> + Send {
        ready(Ok(complete("unused")))
    }
}

impl<Source> ToolCallSession for Session<Source>
where
    Source: Simulated,
{
    type Error = SessionError<Source>;

    async fn call_tool<Host>(
        &mut self,
        request: &CallRequest,
        progress: &ProgressForwarder<'_, Host>,
        _allow_input_required: bool,
    ) -> Result<ToolCallResult, SessionError<Source>>
    where
        Host: ProgressHost + Sync,
    {
        let Some(scenario) = self.script.call.clone() else {
            return Ok(complete(request.name.as_str()));
        };
        if self.script.cancel {
            self.script.cancelled.cancel();
            pending::<()>().await;
        }
        if let Some(error) = self.script.call_error.clone() {
            return Err(SessionError::Runtime(Arc::new(error)));
        }
        if matches!(scenario.input_required, InputRequirement::Reject) {
            let error = SessionError::Runtime(Arc::new(Source::simulated(
                "the server returned an input_required result",
            )));
            return Err(error);
        }
        for event in &scenario.progress {
            progress.report(event.progress, event.total, None).await;
        }
        if matches!(scenario.input_required, InputRequirement::Accept) {
            return Ok(ToolCallResult::InputRequired(InputRequiredResult {
                input_requests: None,
                request_state: Some(scenario.request_state.clone()),
                result_type: "input_required".to_owned(),
                meta: None,
            }));
        }
        Ok(complete(scenario.result_text.as_str()))
    }
}
