use betterlite_logging::logging_pre_post::error::PrePostError;

use super::config::{Headers, Params, VideoLogging};
use super::error::VideoClientError;
use crate::api::vector_store_http_write::error::{BaseLlmException, FailureFacts, handle_error};
use crate::api::vector_store_http_write::service::WriteTransport;

// The B173 client factories: the sync form ignores the provider, and the async form is keyed by it
pub trait VideoClientSource: Send + Sync {
    type Client: WriteTransport;

    fn sync_client(&self, ssl_verify: Option<bool>) -> Result<Self::Client, VideoClientError>;

    fn async_client(
        &self,
        provider: &str,
        ssl_verify: Option<bool>,
    ) -> Result<Self::Client, VideoClientError>;
}

// A supplied client is used only when its mode matches the handler's, and otherwise one is built
pub enum Selected<'call, Client> {
    Supplied(&'call Client),
    Created(Client),
}

impl<Client> Selected<'_, Client> {
    pub(crate) const fn get(&self) -> &Client {
        match self {
            Self::Supplied(client) => client,
            Self::Created(client) => client,
        }
    }
}

// The additional_args LiteLLM's pre_call receives; api_key rides along because B249 records none of it
pub struct VideoPreCall<'call> {
    pub input: &'call str,
    pub api_key: &'call str,
    pub complete_input_dict: &'call Params,
    pub api_base: &'call str,
    pub headers: &'call Headers,
    pub video_id: &'call str,
}

// B249's logging handle: the pre_call it runs and the logging object the provider transforms read
pub trait VideoLoggingHandle: Send + Sync {
    fn pre_call(&self, call: &VideoPreCall<'_>) -> Result<(), PrePostError>;

    fn logging_object(&self) -> VideoLogging;
}

// B2402's shared converter, so a test can stand in for it and watch the facts it is given
pub trait VideoErrorConvert: Send + Sync {
    fn convert(&self, facts: &FailureFacts) -> BaseLlmException;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SharedErrorConvert;

impl VideoErrorConvert for SharedErrorConvert {
    fn convert(&self, facts: &FailureFacts) -> BaseLlmException {
        handle_error(facts, None)
    }
}
