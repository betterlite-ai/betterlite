use std::net::SocketAddr;

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::ConnectInfo;
use axum::http::header::{CONTENT_LENGTH, CONTENT_TYPE, HeaderValue};
use axum::http::{HeaderMap, Method, Request, Uri};
use betterlite_anthropic::native_compaction::protocol::{CompactionProtocol, Mapping};
use betterlite_logging::logging_privacy::PrivacyContext;
use betterlite_models::decisions::Json;
use betterlite_router::compaction_executor::{
    CompactionExecutor, CompactionExecutorError, native_compaction_call,
};
use tokio::task::JoinSet;
use tower::ServiceExt as _;

use super::error::LoopbackError;
use super::headers::child_headers;
use super::target::child_uri;

mod tests;

const MAX_CHILD_BODY_BYTES: usize = 4 * 1024 * 1024;

pub struct LoopbackExecutor {
    pub(crate) app: Router,
    pub(crate) parent_uri: Uri,
    pub(crate) root_path: String,
    pub(crate) headers: HeaderMap,
    pub(crate) peer: Option<SocketAddr>,
}

#[async_trait]
impl CompactionExecutor for LoopbackExecutor {
    async fn execute(
        &self,
        protocol: CompactionProtocol,
        payload: &Mapping,
        parent_model: Option<&str>,
        logging_disabled: bool,
    ) -> Result<Mapping, CompactionExecutorError> {
        let Some(Json::Text(model)) = payload.get("model") else {
            return Err(CompactionExecutorError(Box::new(
                LoopbackError::MissingModel,
            )));
        };
        let child = child_uri(&self.parent_uri, &self.root_path, protocol)
            .map_err(|source| CompactionExecutorError(Box::new(source)))?;
        let headers = child_headers(&self.headers, logging_disabled);
        let payload_bytes = serde_json::to_vec(payload)
            .map_err(|source| CompactionExecutorError(Box::new(source)))?;
        let request = child_request(child, headers, payload_bytes);
        let app = self.app.clone();
        let peer = self.peer;
        let compactor = model.clone();
        let parent = parent_model.map(str::to_owned);
        let mut tasks = JoinSet::new();
        tasks.spawn(async move {
            let call = call_child(app, request, peer, logging_disabled);
            native_compaction_call(parent.as_deref(), Some(compactor.as_str()), call).await
        });
        match tasks.join_next().await {
            Some(Ok(result)) => result,
            Some(Err(source)) => Err(CompactionExecutorError(Box::new(source))),
            None => Err(CompactionExecutorError(Box::new(ChildTaskMissing))),
        }
    }
}

fn child_request(uri: Uri, headers: HeaderMap, payload_bytes: Vec<u8>) -> Request<Body> {
    let length = HeaderValue::from(payload_bytes.len());
    let mut request = Request::new(Body::from(payload_bytes));
    *request.method_mut() = Method::POST;
    *request.uri_mut() = uri;
    *request.headers_mut() = headers;
    request
        .headers_mut()
        .entry(CONTENT_LENGTH)
        .or_insert(length);
    request
        .headers_mut()
        .entry(CONTENT_TYPE)
        .or_insert(HeaderValue::from_static("application/json"));
    request
}

async fn call_child(
    app: Router,
    mut request: Request<Body>,
    peer: Option<SocketAddr>,
    logging_disabled: bool,
) -> Result<Mapping, CompactionExecutorError> {
    // LiteLLM gives the child a fresh privacy context, so only this call's flag is set
    request.extensions_mut().insert(PrivacyContext {
        message_logging_disabled: logging_disabled,
    });
    if let Some(address) = peer {
        request.extensions_mut().insert(ConnectInfo(address));
    }
    let response = match app.oneshot(request).await {
        Ok(response) => response,
        Err(never) => match never {},
    };
    let status = response.status();
    if !status.is_success() {
        return Err(CompactionExecutorError(Box::new(
            LoopbackError::ChildFailed {
                status: status.as_u16(),
            },
        )));
    }
    let body = to_bytes(response.into_body(), MAX_CHILD_BODY_BYTES)
        .await
        .map_err(|source| CompactionExecutorError(Box::new(source)))?;
    // LiteLLM drops the JSON parse error and answers its own fixed message
    serde_json::from_slice(&body)
        .map_err(|_error| CompactionExecutorError(Box::new(LoopbackError::InvalidChildBody)))
}

#[derive(Debug, thiserror::Error)]
#[error("the native compaction child task returned no result")]
struct ChildTaskMissing;
