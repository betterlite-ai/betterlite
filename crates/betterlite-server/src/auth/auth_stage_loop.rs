use std::mem;

use axum::body::{Body, Bytes};
use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;
use secrecy::SecretString;

use super::{prefetch_stage, resolver_for, stage_input};
use crate::auth::auth_errors::FailureContext;
use crate::auth::auth_identity::{
    AuthBackend, AuthContext, INSTALLED_STAGES, StageFailure, dispatch,
};
use crate::auth::body_buffering::{BufferError, max_body_bytes, read_body};
use crate::auth::failures::{body_too_large_response, unreadable_body_response};

// The stages borrow the request only while building their input, so the loop owns it across every await.
pub(crate) async fn run_stages(
    mut request: Request<Body>,
    master_key: &SecretString,
    failure_response: fn(&StageFailure) -> Response,
) -> Result<(Request<Body>, Option<AuthContext>, Option<Bytes>), Box<Response>> {
    let resolver = resolver_for(&request);
    let mut authenticated: Option<AuthContext> = None;
    let mut body: Option<Bytes> = None;
    for backend in INSTALLED_STAGES {
        // the policy and safety stages read the body; only an authenticated caller makes them buffer anything
        if matches!(
            *backend,
            AuthBackend::AuthRequestPolicy | AuthBackend::RequestBodySafety
        ) && body.is_none()
        {
            let limit = max_body_bytes(&request);
            let stream = mem::replace(request.body_mut(), Body::empty());
            body = Some(match read_body(stream, limit).await {
                Ok(bytes) => bytes,
                Err(error) => return Err(Box::new(buffer_failure_response(error, limit))),
            });
        }
        // the input is built before any await, so no borrow of the request lives across one
        let staged = stage_input(
            *backend,
            &request,
            authenticated.as_ref(),
            &resolver,
            body.as_ref(),
        );
        match async {
            let prefetched = prefetch_stage(staged?).await?;
            dispatch(*backend, &prefetched, master_key)
        }
        .await
        {
            Ok(context) => authenticated = Some(context),
            Err(failure) => {
                let context = FailureContext::of(&request, body.as_ref());
                let response = Box::pin(context.respond(&failure, failure_response)).await;
                return Err(Box::new(response));
            }
        }
    }
    Ok((request, authenticated, body))
}

fn buffer_failure_response(error: BufferError, limit: usize) -> Response {
    match error {
        BufferError::TooLarge { observed } => {
            tracing::debug!(
                observed,
                limit,
                "the request body exceeds the auth-time buffering bound"
            );
            body_too_large_response(observed, limit)
        }
        BufferError::Unreadable(reason) => {
            tracing::debug!(error = %reason, "the request body could not be buffered");
            unreadable_body_response()
        }
    }
}

pub(crate) async fn forward(
    mut request: Request<Body>,
    authenticated: Option<AuthContext>,
    body: Option<Bytes>,
    next: Next,
) -> Response {
    if let Some(context) = authenticated {
        request.extensions_mut().insert(context);
    }
    if let Some(bytes) = body {
        *request.body_mut() = Body::from(bytes);
    }
    next.run(request).await
}
