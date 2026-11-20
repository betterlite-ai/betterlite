use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::http::Request;
use betterlite_core::api::api_boundaries::ApiLimits;
use futures_util::StreamExt as _;

use crate::middleware::request_size_limit::SizeLimitLayer;
use crate::state::ServerState;

// the auth read runs before any route limit, so the configured size or the default JSON body size bounds it
pub(crate) fn max_body_bytes(request: &Request<Body>) -> usize {
    let configured = request
        .extensions()
        .get::<Arc<ServerState>>()
        .and_then(|state| SizeLimitLayer::from_settings(&state.config.sections.request_size_limit));
    configured.map_or_else(
        || ApiLimits::default().max_json_bytes,
        |limit| usize::try_from(limit.max_bytes).unwrap_or(usize::MAX),
    )
}

pub(crate) enum BufferError {
    TooLarge { observed: usize },
    Unreadable(axum::Error),
}

pub(crate) async fn read_body(body: Body, limit: usize) -> Result<Bytes, BufferError> {
    let mut stream = body.into_data_stream();
    let mut buffered: Vec<u8> = Vec::new();
    while let Some(frame) = stream.next().await {
        let chunk = frame.map_err(BufferError::Unreadable)?;
        let observed = buffered.len().saturating_add(chunk.len());
        if observed > limit {
            return Err(BufferError::TooLarge { observed });
        }
        buffered.extend_from_slice(&chunk);
    }
    Ok(Bytes::from(buffered))
}
