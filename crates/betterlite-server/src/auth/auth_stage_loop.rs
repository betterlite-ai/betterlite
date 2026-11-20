use std::mem;

use axum::body::{Body, Bytes};
use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;
use secrecy::SecretString;

use super::{prefetch_tag_budgets, resolver_for, stage_input};
use crate::auth::auth_identity::{
    AuthBackend, AuthContext, INSTALLED_STAGES, StageInput, dispatch,
};
use crate::auth::body_buffering::{BufferError, max_body_bytes, read_body};
use crate::auth::failures::{
    body_too_large_response, stage_failure_response, unreadable_body_response,
};

// The stages borrow the request only while building their input, so the loop owns it across every await.
pub(crate) async fn run_stages(
    mut request: Request<Body>,
    master_key: &SecretString,
) -> Result<(Request<Body>, Option<AuthContext>, Option<Bytes>), Box<Response>> {
    let resolver = resolver_for(&request);
    let mut authenticated: Option<AuthContext> = None;
    let mut body: Option<Bytes> = None;
    for backend in INSTALLED_STAGES {
        // the safety stage is the one body reader; only an authenticated caller makes it buffer anything
        if *backend == AuthBackend::RequestBodySafety && body.is_none() {
            let limit = max_body_bytes(&request);
            let stream = mem::replace(request.body_mut(), Body::empty());
            body = Some(match read_body(stream, limit).await {
                Ok(bytes) => bytes,
                Err(error) => return Err(Box::new(buffer_failure_response(error, limit))),
            });
        }
        let input = match stage_input(
            *backend,
            &request,
            authenticated.as_ref(),
            &resolver,
            body.as_ref(),
        ) {
            Ok(input) => input,
            Err(failure) => return Err(Box::new(stage_failure_response(&failure))),
        };
        match run_stage(*backend, input, master_key).await {
            Ok(context) => authenticated = Some(context),
            Err(response) => return Err(response),
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

async fn run_stage(
    backend: AuthBackend,
    input: StageInput,
    master_key: &SecretString,
) -> Result<AuthContext, Box<Response>> {
    let prefetched = prefetch_tag_budgets(input)
        .await
        .map_err(|failure| Box::new(stage_failure_response(&failure)))?;
    dispatch(backend, &prefetched, master_key)
        .map_err(|failure| Box::new(stage_failure_response(&failure)))
}
