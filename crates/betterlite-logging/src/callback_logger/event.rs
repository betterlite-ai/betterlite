use std::sync::Arc;

use betterlite_models::prompt_contracts::StandardCallbackDynamicParams;

use crate::fields::payload::StandardLoggingPayload;

#[derive(Clone, Debug)]
pub struct LoggerEvent {
    pub payload: Arc<StandardLoggingPayload>,
    pub dynamic_params: Arc<StandardCallbackDynamicParams>,
    pub stream: bool,
}
