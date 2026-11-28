use std::sync::Arc;

use betterlite_config::sections::logging_privacy::Settings;

use super::handler::SuccessHandler;
use super::ports::{StandardPayloadBuilder, StreamAssembler, SuccessClock};
use crate::registry::CallbackRegistry;

// Everything a success callback run borrows, owned so a queued job or a settled poll outlives its request
#[derive(Clone)]
pub struct OwnedSuccessHandler {
    pub registry: Arc<CallbackRegistry>,
    pub payload_builder: Arc<dyn StandardPayloadBuilder>,
    pub stream_assembler: Arc<dyn StreamAssembler>,
    pub clock: Arc<dyn SuccessClock>,
    pub excluded_fields: Vec<String>,
    pub known_callback: Arc<dyn Fn(&str) -> bool + Send + Sync>,
    pub privacy: Settings,
}

impl<'handler> From<&'handler OwnedSuccessHandler> for SuccessHandler<'handler> {
    fn from(handler: &'handler OwnedSuccessHandler) -> Self {
        Self {
            registry: handler.registry.as_ref(),
            payload_builder: handler.payload_builder.as_ref(),
            stream_assembler: handler.stream_assembler.as_ref(),
            clock: handler.clock.as_ref(),
            excluded_fields: &handler.excluded_fields,
            known_callback: handler.known_callback.as_ref(),
            privacy: handler.privacy,
        }
    }
}
