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
use axum::routing::{any, post};
use betterlite_prompts::arguments::json::Object;

use crate::auth::auth_identity::AuthContext;

use self::operation::Operation;
use self::ports::{GenaiRoutesState, GenerateContentCall, TokenCountRequest};

// FastAPI's {model_name:path} runs to the literal suffix and a model name may carry slashes, so the catch-all takes the rest
const V1BETA_MODELS: &str = "/v1beta/models/{*rest}";
const MODELS: &str = "/models/{*rest}";

// which of the two dispatch methods a path selects
#[derive(Clone, Copy)]
enum Pipeline {
    Generate,
    Stream,
}

// the six POST routes: both prefixes reach one handler, and a wrong method on a known suffix is Starlette's 405
pub fn routes(state: GenaiRoutesState) -> Router {
    // any() adds the fallback for every other method and skips axum's own Allow header, so the 404 stays bare
    let models = post(handle).merge(any(method_not_allowed));
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
            let call = GenerateContentCall {
                model,
                data: &data,
                context: &context,
                headers: &headers,
            };
            dispatch(&state, call, Pipeline::Generate).await
        }
        Operation::StreamGenerateContent => {
            request::with_model(&mut data, model);
            request::with_stream_flags(&mut data);
            let call = GenerateContentCall {
                model,
                data: &data,
                context: &context,
                headers: &headers,
            };
            dispatch(&state, call, Pipeline::Stream).await
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

async fn dispatch(
    state: &GenaiRoutesState,
    call: GenerateContentCall<'_>,
    pipeline: Pipeline,
) -> Response {
    let outcome = match pipeline {
        Pipeline::Generate => state.generation.generate(call).await,
        Pipeline::Stream => state.generation.stream(call).await,
    };
    match outcome {
        Ok(answer) => response::generation(answer),
        Err(exception) => error::failure_response(&exception, state.include_call_id_in_error_body),
    }
}

async fn count_tokens(state: &GenaiRoutesState, model: &str, data: &Object) -> Response {
    let contents = match request::contents(data) {
        Ok(contents) => contents,
        Err(exception) => {
            return error::failure_response(&exception, state.include_call_id_in_error_body);
        }
    };
    let messages = match state.translator.messages(model, &contents) {
        Ok(messages) => messages,
        Err(exception) => {
            return error::failure_response(&exception, state.include_call_id_in_error_body);
        }
    };
    let request = TokenCountRequest {
        model: model.to_owned(),
        contents,
        messages,
    };
    match state.token_counter.count_tokens(&request, true).await {
        Ok(counted) => response::token_count(counted),
        Err(exception) => error::failure_response(&exception, state.include_call_id_in_error_body),
    }
}
