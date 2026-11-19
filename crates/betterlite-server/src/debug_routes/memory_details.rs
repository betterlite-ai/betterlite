use axum::extract::Extension;
use axum::http::Uri;
use axum::response::{IntoResponse as _, Response};

use crate::auth::auth_identity::AuthContext;
use crate::debug_routes::query::{self, QueryError};
use crate::debug_routes::unsupported::unsupported_runtime;
use crate::tracing_routes::router::unauthenticated;

mod tests;

const DEFAULT_TOP_N: i64 = 20;
const DEFAULT_INCLUDE_PROCESS_INFO: bool = true;

pub(super) fn parse(uri: &Uri) -> Result<(i64, bool), QueryError> {
    let values = query::values(uri);
    let top_n = query::integer(&values, "top_n", DEFAULT_TOP_N)?;
    let include_process_info = query::boolean(
        &values,
        "include_process_info",
        DEFAULT_INCLUDE_PROCESS_INFO,
    )?;
    Ok((top_n, include_process_info))
}

pub(super) async fn handler(caller: Option<Extension<AuthContext>>, uri: Uri) -> Response {
    if caller.is_none() {
        return unauthenticated();
    }
    if let Err(error) = parse(&uri) {
        return error.into_response();
    }
    unsupported_runtime("the native runtime cannot measure Python object memory")
}
