use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path, Query, State};
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse as _, Response};
use betterlite_logging::trace_types::TraceScope;
use betterlite_tracing::storage::trace_store::SpanErrorTarget;
use serde::Deserialize;

use crate::auth::auth_identity::AuthContext;
use crate::tracing_routes::TraceRoutesState;
use crate::tracing_routes::access::{TraceAuth, provide_trace_access};
use crate::tracing_routes::receiver::TraceRuntime;
use crate::tracing_routes::router::{
    DetailBody, TraceRefQuery, ValidationBody, ValidationIssue, access_failure, read_failure,
    unauthenticated,
};

const MAX_CURSOR_CHARS: usize = 512;

#[derive(Debug, Deserialize)]
struct ErrorQuery {
    #[serde(default)]
    trace_ref: String,
    cursor: Option<String>,
}

pub(super) fn reader(
    caller: Option<Extension<AuthContext>>,
    state: &TraceRoutesState,
) -> Result<(Arc<dyn TraceRuntime>, TraceScope), Box<Response>> {
    let Some(Extension(context)) = caller else {
        return Err(Box::new(unauthenticated()));
    };
    let auth = TraceAuth::from_principal(&context.principal);
    let access = provide_trace_access(&auth, state.receiver.clone());
    match access.reader() {
        Ok((receiver, scope)) => Ok((Arc::clone(receiver), scope.clone())),
        Err(error) => Err(Box::new(access_failure(&error))),
    }
}

pub(super) async fn get_trace(
    State(app): State<TraceRoutesState>,
    Path(trace_id): Path<String>,
    identity: Option<Extension<AuthContext>>,
    request_uri: Uri,
) -> Response {
    let (receiver, scope) = match reader(identity, &app) {
        Ok(pair) => pair,
        Err(response) => return *response,
    };
    match receiver
        .get_trace(&trace_id, &scope, &trace_ref(&request_uri))
        .await
    {
        Ok(Some(trace)) => Json(trace).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(DetailBody {
                detail: format!("Trace {trace_id} not found"),
            }),
        )
            .into_response(),
        Err(error) => read_failure(&error),
    }
}

pub(super) async fn get_span(
    State(state): State<TraceRoutesState>,
    uri: Uri,
    caller: Option<Extension<AuthContext>>,
    Path(ids): Path<(String, String)>,
) -> Response {
    let (receiver, scope) = match reader(caller, &state) {
        Ok(pair) => pair,
        Err(response) => return *response,
    };
    let reference = trace_ref(&uri);
    match receiver.get_span(&ids.0, &ids.1, &scope, &reference).await {
        Err(error) => read_failure(&error),
        Ok(found) => match found {
            Some(span) => Json(span).into_response(),
            None => (
                StatusCode::NOT_FOUND,
                Json(DetailBody {
                    detail: format!("Span {} not found", ids.1),
                }),
            )
                .into_response(),
        },
    }
}

pub(super) async fn get_span_error(
    State(state): State<TraceRoutesState>,
    caller: Option<Extension<AuthContext>>,
    Path((trace_id, span_id)): Path<(String, String)>,
    uri: Uri,
) -> Response {
    let Ok(Query(params)) = Query::<ErrorQuery>::try_from_uri(&uri) else {
        let body = DetailBody {
            detail: "invalid trace query parameters".to_owned(),
        };
        return (StatusCode::BAD_REQUEST, Json(body)).into_response();
    };
    if let Some(response) = cursor_limit(params.cursor.as_deref()) {
        return response;
    }
    let (receiver, scope) = match reader(caller, &state) {
        Ok(pair) => pair,
        Err(response) => return *response,
    };
    let target = SpanErrorTarget {
        trace_id: &trace_id,
        span_id: &span_id,
        trace_ref: &params.trace_ref,
    };
    match receiver
        .get_span_error(target, &scope, params.cursor.as_deref())
        .await
    {
        Ok(Some(page)) => Json(page).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(DetailBody {
                detail: "Span diagnostic not found or no longer available".to_owned(),
            }),
        )
            .into_response(),
        Err(error) => read_failure(&error),
    }
}

fn trace_ref(uri: &Uri) -> String {
    Query::<TraceRefQuery>::try_from_uri(uri)
        .map_or_else(|_rejection| String::new(), |Query(params)| params.trace_ref)
}

fn cursor_limit(cursor: Option<&str>) -> Option<Response> {
    if cursor.is_some_and(|value| value.chars().count() > MAX_CURSOR_CHARS) {
        let issue = ValidationIssue {
            kind: "string_too_long",
            loc: vec!["query".to_owned(), "cursor".to_owned()],
            msg: "String should have at most 512 characters",
        };
        let body = ValidationBody {
            detail: vec![issue],
        };
        return Some((StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response());
    }
    None
}
