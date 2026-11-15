use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::{Extension, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use serde::Deserialize;

use crate::auth::auth_identity::AuthContext;
use crate::error_body::json_error;
use crate::tracing_routes::TraceRoutesState;
use crate::tracing_routes::access::TraceAuth;
use crate::tracing_routes::query::{QueryAccessError, QueryRunError, provide_trace_query_access};
use crate::tracing_routes::router::{DetailBody, ValidationBody, ValidationIssue, unauthenticated};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TraceQueryRequest {
    sql: String,
}

pub(super) async fn query_traces(
    State(state): State<TraceRoutesState>,
    caller: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Response {
    let Some(Extension(context)) = caller else {
        return unauthenticated();
    };
    let auth = TraceAuth::from_principal(&context.principal);
    let access =
        match provide_trace_query_access(&auth, state.receiver.as_ref(), state.master_key.as_ref())
        {
            Ok(access) => access,
            Err(error) => return query_access_failure(&error),
        };
    let Ok(request) = serde_json::from_slice::<TraceQueryRequest>(&body) else {
        let issue = ValidationIssue {
            kind: "value_error",
            loc: vec!["body".to_owned()],
            msg: "invalid trace query request",
        };
        let reply = ValidationBody {
            detail: vec![issue],
        };
        return (StatusCode::UNPROCESSABLE_ENTITY, Json(reply)).into_response();
    };
    match access
        .store
        .query_sql(&request.sql, &access.scope, &access.secret)
        .await
    {
        Ok(result) => raw_json_response(result),
        Err(error) => query_run_failure(&error),
    }
}

pub(super) async fn query_help(
    State(state): State<TraceRoutesState>,
    caller: Option<Extension<AuthContext>>,
) -> Response {
    let Some(Extension(context)) = caller else {
        return unauthenticated();
    };
    let auth = TraceAuth::from_principal(&context.principal);
    let access =
        match provide_trace_query_access(&auth, state.receiver.as_ref(), state.master_key.as_ref())
        {
            Ok(access) => access,
            Err(error) => return query_access_failure(&error),
        };
    match access.store.query_help(&access.scope, &access.secret).await {
        Ok(result) => raw_json_response(result),
        Err(QueryRunError::Unavailable(message)) => {
            tracing::warn!(error = %message, "trace query help unavailable");
            let body = DetailBody {
                detail: "Trace query help is temporarily unavailable".to_owned(),
            };
            (StatusCode::SERVICE_UNAVAILABLE, Json(body)).into_response()
        }
        Err(QueryRunError::Invalid(_message)) => json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal server error",
            "internal_server_error",
            "500",
        ),
    }
}

fn raw_json_response(body: String) -> Response {
    let mut response = Response::new(Body::from(body));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
}

fn query_access_failure(error: &QueryAccessError) -> Response {
    let status = match error {
        QueryAccessError::SecretMissing => StatusCode::SERVICE_UNAVAILABLE,
        QueryAccessError::ProjectKeyRequired | QueryAccessError::ScopeRequired => {
            StatusCode::FORBIDDEN
        }
        QueryAccessError::Unavailable => StatusCode::NOT_IMPLEMENTED,
    };
    let body = DetailBody {
        detail: error.to_string(),
    };
    (status, Json(body)).into_response()
}

fn query_run_failure(error: &QueryRunError) -> Response {
    match error {
        QueryRunError::Invalid(message) => {
            let body = DetailBody {
                detail: message.clone(),
            };
            (StatusCode::BAD_REQUEST, Json(body)).into_response()
        }
        QueryRunError::Unavailable(message) => {
            tracing::warn!(error = %message, "trace SQL query unavailable");
            let body = DetailBody {
                detail: "Trace SQL query failed or exceeded reader limits".to_owned(),
            };
            (StatusCode::SERVICE_UNAVAILABLE, Json(body)).into_response()
        }
    }
}
