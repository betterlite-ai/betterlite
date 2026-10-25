use std::future::Future;
use std::pin::Pin;

use futures_core::Stream;

use crate::context::SessionContext;
use crate::errors::HarnessError;
use crate::types::{Event, Harness};

pub trait BaseHarnessHandler: Send {
    fn harness(&self) -> Harness;
    fn start(
        &mut self,
        context: &mut SessionContext,
    ) -> Pin<Box<dyn Future<Output = Result<(), HarnessError>> + Send + '_>>;
    fn turn(
        &mut self,
        context: &mut SessionContext,
        prompt: String,
    ) -> Pin<Box<dyn Stream<Item = Event> + Send + '_>>;
    fn stop(
        &mut self,
        context: &mut SessionContext,
    ) -> Pin<Box<dyn Future<Output = Result<(), HarnessError>> + Send + '_>>;
    fn native_session_id(&self) -> Option<String>;
    fn resume(
        &mut self,
        context: &mut SessionContext,
        native_session_id: String,
    ) -> Pin<Box<dyn Future<Output = Result<(), HarnessError>> + Send + '_>>;
    fn history(&self, _context: &SessionContext) -> Result<Vec<serde_json::Value>, HarnessError> {
        Err(HarnessError::CapabilityUnsupported {
            message: format!(
                "Harness.{} does not expose history",
                match self.harness() {
                    Harness::ClaudeCode => "CLAUDE_CODE",
                    Harness::Codex => "CODEX",
                    Harness::OpenCode => "OPENCODE",
                    Harness::DeepAgents => "DEEPAGENTS",
                }
            ),
        })
    }
}
