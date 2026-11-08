use async_trait::async_trait;
use axum::body::{Body, Bytes};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use betterlite_config::sections::settings::JsonValue;
use betterlite_http::passthrough::get_response_headers;
use indexmap::IndexMap;

const PASSTHROUGH_ROUTE_TYPE: &str = "allm_passthrough_route";

pub struct PassthroughCall<Key, Router> {
    pub record: IndexMap<String, JsonValue>,
    pub request: Parts,
    pub response_headers: HeaderMap,
    pub key: Key,
    pub router: Option<Router>,
    pub model: Option<String>,
}

pub struct BaseProcessCall<'call, Key, Router> {
    pub record: &'call IndexMap<String, JsonValue>,
    pub request: &'call Parts,
    pub response_headers: &'call HeaderMap,
    pub key: &'call Key,
    pub router: Option<&'call Router>,
    pub model: Option<&'call str>,
    pub route_type: &'call str,
}

pub enum ProcessedReply {
    Streaming(Response),
    Direct(Response),
    Upstream(UpstreamReply),
}

pub struct UpstreamReply {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
}

#[async_trait]
pub trait LlmRequestProcessor<Key, Router> {
    type Error;

    async fn base_process_llm_request(
        &self,
        call: BaseProcessCall<'_, Key, Router>,
    ) -> Result<ProcessedReply, Self::Error>;
}

#[async_trait]
pub trait ExceptionMapper<Key, Error> {
    type Mapped;

    async fn handle_llm_api_exception(&self, error: Error, key: &Key) -> Self::Mapped;
}

#[async_trait]
pub trait PassthroughProcessor<Key, Router> {
    type Error;

    async fn process_passthrough(
        &self,
        call: PassthroughCall<Key, Router>,
    ) -> Result<Response, Self::Error>;
}

pub struct LlmPassthroughProcessor<Processor, Mapper> {
    pub processor: Processor,
    pub exceptions: Mapper,
}

#[async_trait]
impl<Key, Router, Processor, Mapper> PassthroughProcessor<Key, Router>
    for LlmPassthroughProcessor<Processor, Mapper>
where
    Key: Send + Sync + 'static,
    Router: Send + Sync + 'static,
    Processor: LlmRequestProcessor<Key, Router> + Send + Sync,
    Mapper: ExceptionMapper<Key, Processor::Error> + Send + Sync,
    Processor::Error: Send,
    Mapper::Mapped: Send,
{
    type Error = Mapper::Mapped;

    async fn process_passthrough(
        &self,
        call: PassthroughCall<Key, Router>,
    ) -> Result<Response, Self::Error> {
        let processed = self
            .processor
            .base_process_llm_request(BaseProcessCall {
                record: &call.record,
                request: &call.request,
                response_headers: &call.response_headers,
                key: &call.key,
                router: call.router.as_ref(),
                model: call.model.as_deref(),
                route_type: PASSTHROUGH_ROUTE_TYPE,
            })
            .await;
        match processed {
            Ok(ProcessedReply::Streaming(response) | ProcessedReply::Direct(response)) => {
                Ok(response)
            }
            Ok(ProcessedReply::Upstream(upstream)) => {
                let headers =
                    get_response_headers(&upstream.headers, None, Some(&call.response_headers));
                let mut response = Response::new(Body::from(upstream.body));
                *response.status_mut() = upstream.status;
                *response.headers_mut() = headers;
                Ok(response)
            }
            Err(error) => {
                let mapped = self
                    .exceptions
                    .handle_llm_api_exception(error, &call.key)
                    .await;
                Err(mapped)
            }
        }
    }
}

mod tests;
