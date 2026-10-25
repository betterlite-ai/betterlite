use std::future::Future;
use std::pin::Pin;

use crate::context::SessionContext;
use crate::errors::CapabilityUnsupported;
use crate::transformation::BaseHarnessConfig;
use crate::types::Event;

pub trait BaseHarnessHandler: Send {
    fn config(&self) -> &dyn BaseHarnessConfig;
    fn start(
        &mut self,
        context: &mut SessionContext,
    ) -> Pin<Box<dyn Future<Output = Result<(), crate::errors::HarnessError>> + Send + '_>>;
    fn turn(
        &mut self,
        context: &mut SessionContext,
        prompt: String,
    ) -> Pin<Box<dyn Iterator<Item = Event> + Send>>;
    fn stop(
        &mut self,
        context: &mut SessionContext,
    ) -> Pin<Box<dyn Future<Output = Result<(), crate::errors::HarnessError>> + Send + '_>>;
    fn native_session_id(&self) -> Option<String>;
    fn resume(
        &mut self,
        context: &mut SessionContext,
        native_session_id: String,
    ) -> Pin<Box<dyn Future<Output = Result<(), crate::errors::HarnessError>> + Send + '_>>;
    fn history(
        &self,
        _context: &SessionContext,
    ) -> Result<Vec<serde_json::Value>, CapabilityUnsupported> {
        Err(CapabilityUnsupported(format!(
            "Harness.{} does not expose history",
            self.config().harness().value()
        )))
    }
}
