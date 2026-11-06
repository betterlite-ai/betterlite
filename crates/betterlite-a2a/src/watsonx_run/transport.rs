use std::collections::BTreeMap;
use std::error::Error;
use std::future::Future;
use std::time::Duration;

use super::run_api::RunBody;

#[derive(Debug)]
pub struct RunRequest<'request> {
    pub url: &'request str,
    pub headers: &'request BTreeMap<String, String>,
    pub body: Option<&'request RunBody>,
    pub timeout: Duration,
}

#[derive(Debug)]
pub struct RunResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

#[derive(Debug)]
pub struct StreamResponse<E> {
    pub status: u16,
    pub content_type: String,
    pub body: StreamBody<E>,
}

#[derive(Debug)]
pub enum StreamBody<E> {
    Chunks(Vec<Vec<u8>>),
    ReadFailed(E),
}

pub trait RunTransport: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn post(
        &self,
        request: &RunRequest<'_>,
    ) -> impl Future<Output = Result<RunResponse, Self::Error>> + Send;

    fn get(
        &self,
        request: &RunRequest<'_>,
    ) -> impl Future<Output = Result<RunResponse, Self::Error>> + Send;

    fn post_stream(
        &self,
        request: &RunRequest<'_>,
    ) -> impl Future<Output = Result<StreamResponse<Self::Error>, Self::Error>> + Send;

    fn pause(&self, duration: Duration) -> impl Future<Output = ()> + Send;
}
