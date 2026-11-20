use std::error::Error;
use std::future::Future;
use std::time::Duration;

use super::request::KillSwitchRequest;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KillSwitchSendOptions {
    pub stream: bool,
    pub follow_redirects: bool,
}

pub trait KillSwitchHttpClient: Send + Sync {
    type Error: Error + Send + Sync + 'static;
    type Request: Send + 'static;
    type Response: KillSwitchResponse<Error = Self::Error> + 'static;

    fn build_request(
        &self,
        request: &KillSwitchRequest,
        timeout: Duration,
    ) -> Result<Self::Request, Self::Error>;

    fn send(
        &self,
        request: Self::Request,
        options: KillSwitchSendOptions,
    ) -> impl Future<Output = Result<Self::Response, Self::Error>> + Send;
}

pub trait KillSwitchResponse: Send {
    type Error: Error + Send + Sync + 'static;

    fn status(&self) -> u16;

    fn next_text_chunk(
        &mut self,
    ) -> impl Future<Output = Option<Result<String, Self::Error>>> + Send;

    fn close(&mut self) -> impl Future<Output = Result<(), Self::Error>> + Send;
}
