use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::Body;
use axum::extract::State;
use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;

use super::body::GuardedBody;

#[derive(Debug, Default)]
pub struct InFlightRequests {
    pub current: AtomicU64,
}

struct InFlightGuard {
    requests: Arc<InFlightRequests>,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        super::decrement(&self.requests.current);
    }
}

pub(super) async fn track_in_flight(
    State(requests): State<Arc<InFlightRequests>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if super::is_websocket_upgrade(&request) {
        return next.run(request).await;
    }
    requests.current.fetch_add(1, Ordering::AcqRel);
    let guard = InFlightGuard {
        requests: Arc::clone(&requests),
    };
    let response = next.run(request).await;
    let (parts, body) = response.into_parts();
    Response::from_parts(
        parts,
        Body::from_stream(GuardedBody {
            inner: body.into_data_stream(),
            guard: Some(guard),
        }),
    )
}
