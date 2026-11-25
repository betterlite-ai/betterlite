mod request;
mod scheduled;
mod stream;

use std::sync::Arc;

use axum::body::to_bytes;
use axum::extract::{Extension, Query, Request, State};
use axum::response::Response;
use betterlite_config::sections::settings::JsonValue;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_openai::openai_base_config::Object as JsonMap;
use serde::Deserialize;

use self::request::{QueueRequest, build_request};
use self::scheduled::{QueueError, non_streaming, report};
use self::stream::streamed;
use super::Caller;
use super::state::InvitationRouteState;
use crate::tracing_routes::router::unauthenticated;

const BODY_LIMIT: usize = TransportLimits::DEFAULT.body_bytes;

#[derive(Deserialize)]
pub(super) struct QueueQuery {
    model: Option<String>,
}

pub(super) async fn queue_chat_completions(
    State(state): State<Arc<InvitationRouteState>>,
    caller: Caller,
    query: Query<QueueQuery>,
    request: Request,
) -> Response {
    let Some(Extension(context)) = caller else {
        return unauthenticated();
    };
    let (parts, body) = request.into_parts();
    let empty = JsonMap::new();
    let bytes = match to_bytes(body, BODY_LIMIT).await {
        Ok(bytes) => bytes,
        Err(error) => {
            let failure = QueueError::Message(error.to_string());
            let hook = state.queue.failure.as_deref();
            return report(hook, &context.principal, &empty, failure.exception()).await;
        }
    };
    let mut data = JsonMap::new();
    let input = QueueRequest {
        queue: &state.queue,
        caller: &context.principal,
        parts: &parts,
        model: query.model.as_deref(),
    };
    match Box::pin(process(input, &bytes, &mut data)).await {
        Ok(response) => response,
        Err(error) => {
            let hook = state.queue.failure.as_deref();
            report(hook, input.caller, &data, error.exception()).await
        }
    }
}

async fn process(
    input: QueueRequest<'_>,
    body: &[u8],
    data: &mut JsonMap,
) -> Result<Response, QueueError> {
    build_request(input, body, data)?;
    let Some(router) = input.queue.router.as_ref() else {
        return Err(QueueError::NoRouter);
    };
    if matches!(data.get("stream"), Some(JsonValue::Bool(true))) {
        return Box::pin(streamed(input, router, data)).await;
    }
    non_streaming(input, router, data).await
}
