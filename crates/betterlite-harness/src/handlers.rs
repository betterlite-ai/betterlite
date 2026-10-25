mod tests;

use std::future::Future;

use futures_core::Stream;

use crate::context::SessionContext;
use crate::errors::HarnessError;
use crate::events::Event;
use crate::sandbox::Sandbox;
use crate::transformation::HarnessConfig;
use crate::types::{Capability, JsonObject};

pub trait HarnessHandler: Send {
    type Config: HarnessConfig;

    fn config(&self) -> &Self::Config;
    // called again after an interrupt
    fn start<S>(
        &mut self,
        context: &mut SessionContext<S>,
    ) -> impl Future<Output = Result<(), HarnessError>> + Send
    where
        S: Sandbox;
    // never yields Event::Done, and sets context.outcome.final_text and output_json
    fn turn<S>(
        &mut self,
        context: &mut SessionContext<S>,
        prompt: &str,
    ) -> impl Stream<Item = Result<Event, HarnessError>> + Send
    where
        S: Sandbox;
    // safe to call twice
    fn stop<S>(
        &mut self,
        context: &mut SessionContext<S>,
    ) -> impl Future<Output = Result<(), HarnessError>> + Send
    where
        S: Sandbox;
    fn native_session_id(&self) -> Option<String>;
    fn resume<S>(
        &mut self,
        context: &mut SessionContext<S>,
        native_session_id: &str,
    ) -> impl Future<Output = Result<(), HarnessError>> + Send
    where
        S: Sandbox;

    fn history<S>(
        &self,
        _context: &SessionContext<S>,
    ) -> impl Future<Output = Result<Vec<JsonObject>, HarnessError>> + Send
    where
        S: Sync,
    {
        let unsupported = HarnessError::CapabilityUnsupported {
            harness: <Self::Config as HarnessConfig>::HARNESS,
            capability: Capability::History,
        };
        async move { Err(unsupported) }
    }
}
