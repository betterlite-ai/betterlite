use std::sync::Arc;

use axum::Router;
use axum::extract::{Request, State};
use axum::middleware::{Next, from_fn_with_state};
use axum::response::Response;

pub struct LoopbackTarget {
    pub(crate) app: Router,
    pub(crate) scalar_root: String,
}

pub fn install(app: Router, scalar_root: String) -> Router {
    // the target is the stack from before this layer, so a child request carries no target of its own
    let target = Arc::new(LoopbackTarget {
        app: app.clone(),
        scalar_root,
    });
    app.layer(from_fn_with_state(target, insert_target))
}

async fn insert_target(
    State(target): State<Arc<LoopbackTarget>>,
    mut request: Request,
    next: Next,
) -> Response {
    request.extensions_mut().insert(target);
    next.run(request).await
}
