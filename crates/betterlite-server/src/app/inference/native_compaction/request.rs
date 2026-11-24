use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{ConnectInfo, OriginalUri};
use axum::http::request::Parts;

use super::target::LoopbackTarget;
use crate::compaction_loopback::executor::LoopbackExecutor;
use crate::middleware::request_root_paths::get_request_root_path;

#[must_use]
pub fn executor_for(parts: &Parts) -> Option<LoopbackExecutor> {
    let target = parts.extensions.get::<Arc<LoopbackTarget>>()?;
    let parent_uri = parts
        .extensions
        .get::<OriginalUri>()
        .map_or_else(|| parts.uri.clone(), |original| original.0.clone());
    let root_path = get_request_root_path(&parts.extensions, &target.scalar_root);
    let peer = parts
        .extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|connect_info| connect_info.0);
    Some(LoopbackExecutor {
        app: target.app.clone(),
        parent_uri,
        root_path,
        headers: parts.headers.clone(),
        peer,
    })
}
