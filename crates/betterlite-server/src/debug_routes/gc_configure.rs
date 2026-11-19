use axum::extract::Extension;
use axum::http::Uri;
use axum::response::{IntoResponse as _, Response};

use crate::auth::auth_identity::AuthContext;
use crate::debug_routes::query::{self, QueryError};
use crate::debug_routes::unsupported::unsupported_runtime;
use crate::tracing_routes::router::unauthenticated;

mod tests;

const DEFAULT_GENERATION_0: i64 = 700;
const DEFAULT_GENERATION_1: i64 = 10;
const DEFAULT_GENERATION_2: i64 = 10;

pub(super) fn parse(uri: &Uri) -> Result<[i64; 3], QueryError> {
    let values = query::values(uri);
    Ok([
        query::integer(&values, "generation_0", DEFAULT_GENERATION_0)?,
        query::integer(&values, "generation_1", DEFAULT_GENERATION_1)?,
        query::integer(&values, "generation_2", DEFAULT_GENERATION_2)?,
    ])
}

pub(super) async fn handler(caller: Option<Extension<AuthContext>>, uri: Uri) -> Response {
    if caller.is_none() {
        return unauthenticated();
    }
    if let Err(error) = parse(&uri) {
        return error.into_response();
    }
    unsupported_runtime("the native runtime has no Python garbage collector to configure")
}
