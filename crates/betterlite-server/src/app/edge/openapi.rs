use axum::Router;
use axum::body::Body;
use axum::http::{HeaderValue, header};
use axum::response::Response;
use axum::routing::get;
use betterlite_openapi::document_builder::{
    DocumentInputs, RouteDoc, build_document, default_info,
};
use betterlite_openapi::llm_request_bodies::LlmRequestSchemas;
use betterlite_openapi::websocket_stubs::{QueryParam, WebsocketRoute};
use bytes::Bytes;
use thiserror::Error;

mod tests;

const CHAT_COMPLETION_PATHS: [&str; 4] = [
    "/v1/chat/completions",
    "/chat/completions",
    "/engines/{model}/chat/completions",
    "/openai/deployments/{model}/chat/completions",
];

const EMBEDDING_PATHS: [&str; 4] = [
    "/v1/embeddings",
    "/embeddings",
    "/engines/{model}/embeddings",
    "/openai/deployments/{model}/embeddings",
];

const RESPONSES_API_PATHS: [&str; 3] = ["/v1/responses", "/responses", "/openai/v1/responses"];

const REALTIME_QUERY_PARAMS: [&str; 3] = ["model", "intent", "guardrails"];
const RESPONSES_WS_QUERY_PARAMS: [&str; 1] = ["model"];

#[derive(Debug, Error)]
pub enum DocumentError {
    #[error("the OpenAPI document cannot be serialized: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

#[must_use]
pub fn document_inputs(server_root_path: &str, version: &str) -> DocumentInputs {
    DocumentInputs {
        info: default_info(server_root_path, version),
        routes: llm_routes(),
        websockets: websocket_routes(),
        llm_schemas: LlmRequestSchemas {
            chat_completion: None,
            embedding: None,
            responses: None,
        },
        server_root_path: server_root_path.to_owned(),
    }
}

pub fn document_router(openapi_url: Option<&str>, server_root_path: &str, title: &str) -> Router {
    let Some(path) = openapi_url else {
        return Router::new();
    };
    let document = match document_bytes(server_root_path, title) {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::error!(%error, "the OpenAPI document cannot be served");
            return Router::new();
        }
    };
    Router::new().route(
        path,
        get(move || {
            let body = document.clone();
            async move {
                let mut response = Response::new(Body::from(body));
                response.headers_mut().insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                );
                response
            }
        }),
    )
}

fn document_bytes(server_root_path: &str, title: &str) -> Result<Bytes, DocumentError> {
    let mut inputs = document_inputs(server_root_path, env!("CARGO_PKG_VERSION"));
    title.clone_into(&mut inputs.info.title);
    serde_json::to_vec(&build_document(&inputs))
        .map(Bytes::from)
        .map_err(|source| DocumentError::Encode { source })
}

fn llm_routes() -> Vec<RouteDoc> {
    let mut routes = Vec::with_capacity(11);
    for path in CHAT_COMPLETION_PATHS {
        routes.push(post_route(path, "chat_completion", "chat/completions"));
    }
    for path in EMBEDDING_PATHS {
        routes.push(post_route(path, "embeddings", "embeddings"));
    }
    for path in RESPONSES_API_PATHS {
        routes.push(post_route(path, "responses_api", "responses"));
    }
    routes
}

fn post_route(path: &str, operation: &str, category: &str) -> RouteDoc {
    RouteDoc {
        path: path.to_owned(),
        method: "POST".to_owned(),
        name: operation.to_owned(),
        summary: None,
        tags: vec![category.to_owned()],
        parameters: Vec::new(),
        request_body: None,
        error_responses: false,
    }
}

const WEBSOCKET_ROUTES: [(&str, &str, &[&str]); 5] = [
    (
        "/openai/v1/realtime",
        "realtime_websocket_endpoint",
        &REALTIME_QUERY_PARAMS,
    ),
    (
        "/v1/realtime",
        "realtime_websocket_endpoint",
        &REALTIME_QUERY_PARAMS,
    ),
    (
        "/realtime",
        "realtime_websocket_endpoint",
        &REALTIME_QUERY_PARAMS,
    ),
    (
        "/v1/responses",
        "responses_websocket_endpoint",
        &RESPONSES_WS_QUERY_PARAMS,
    ),
    (
        "/responses",
        "responses_websocket_endpoint",
        &RESPONSES_WS_QUERY_PARAMS,
    ),
];

fn websocket_routes() -> Vec<WebsocketRoute> {
    WEBSOCKET_ROUTES
        .iter()
        .map(|(path, operation, query_params)| WebsocketRoute {
            path: (*path).to_owned(),
            name: Some((*operation).to_owned()),
            query_params: query_params
                .iter()
                .map(|declared| QueryParam {
                    name: (*declared).to_owned(),
                    required: false,
                })
                .collect(),
        })
        .collect()
}
