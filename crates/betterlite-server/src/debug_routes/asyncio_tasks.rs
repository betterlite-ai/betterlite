use axum::extract::Extension;
use axum::response::Response;

use crate::auth::auth_identity::AuthContext;
use crate::debug_routes::unsupported::unsupported_runtime;
use crate::tracing_routes::router::unauthenticated;

mod tests;

pub(super) async fn handler(caller: Option<Extension<AuthContext>>) -> Response {
    if caller.is_none() {
        return unauthenticated();
    }
    unsupported_runtime("the native runtime has no Python asyncio event loop to inspect")
}
