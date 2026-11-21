mod error;
mod operation;
pub mod ports;
mod request;
mod response;
mod tests;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Extension, Path, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::post;
use betterlite_prompts::arguments::json::Object;

use crate::auth::auth_identity::AuthContext;

use self::operation::Operation;
use self::ports::{GenaiRoutesState, GenerateContentCall, TokenCountRequest};

// FastAPI's {model_name:path} runs to the literal suffix and a model name may carry slashes, so the catch-all takes the rest
const V1BETA_MODELS: &str = "/v1beta/models/{*rest}";
const MODELS: &str = "/models/{*rest}";

// the six POST routes: both prefixes reach one handler, and a wrong method on a known suffix is Starlette's 405
pub fn routes(state: GenaiRoutesState) -> Router {
    let models = post(handle).method_not_allowed_fallback(method_not_allowed);
    Router::new()
        .route(V1BETA_MODELS, models.clone())
        .route(MODELS, models)
        // axum's extractor default of 2 MB would answer before B1100, which owns the JSON limit and its facts
        .layer(DefaultBodyLimit::disable())
        .with_state(state)
}

async fn handle(
    State(state): State<GenaiRoutesState>,
    Extension(context): Extension<AuthContext>,
    Path(rest): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some((model, operation)) = operation::split(&rest) else {
        return response::not_found();
    };
    let mut data = match request::decode(&body) {
        Ok(data) => data,
        Err(error) => return error::request_body_failure(&error),
    };
    match operation {
        Operation::GenerateContent => {
            request::with_model(&mut data, model);
            generate(&state, &context, &headers, model, &data).await
        }
        Operation::StreamGenerateContent => {
            request::with_model(&mut data, model);
            request::with_stream_flags(&mut data);
            stream(&state, &context, &headers, model, &data).await
        }
        Operation::CountTokens => count_tokens(&state, model, &data).await,
    }
}

async fn method_not_allowed(Path(rest): Path<String>) -> Response {
    if operation::split(&rest).is_some() {
        response::method_not_allowed()
    } else {
        response::not_found()
    }
}

async fn generate(
    state: &GenaiRoutesState,
    context: &AuthContext,
    headers: &HeaderMap,
    model: &str,
    data: &Object,
) -> Response {
    let call = GenerateContentCall {
        model,
        data,
        context,
        headers,
    };
    match state.generation.generate(call).await {
        Ok(answer) => response::generation(answer),
        Err(exception) => error::failure_response(&exception),
    }
}

async fn stream(
    state: &GenaiRoutesState,
    context: &AuthContext,
    headers: &HeaderMap,
    model: &str,
    data: &Object,
) -> Response {
    let call = GenerateContentCall {
        model,
        data,
        context,
        headers,
    };
    match state.generation.stream(call).await {
        Ok(answer) => response::generation(answer),
        Err(exception) => error::failure_response(&exception),
    }
}

async fn count_tokens(state: &GenaiRoutesState, model: &str, data: &Object) -> Response {
    let contents = match request::contents(data) {
        Ok(contents) => contents,
        Err(exception) => return error::failure_response(&exception),
    };
    let messages = match state.translator.messages(model, &contents) {
        Ok(messages) => messages,
        Err(exception) => return error::failure_response(&exception),
    };
    let request = TokenCountRequest {
        model: model.to_owned(),
        contents,
        messages,
    };
    match state.token_counter.count_tokens(&request, true).await {
        Ok(counted) => response::token_count(counted),
        Err(exception) => error::failure_response(&exception),
    }
}
