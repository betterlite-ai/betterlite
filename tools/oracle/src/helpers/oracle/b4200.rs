use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::Request;
use betterlite_config::sections::request_root_paths::Settings;
use betterlite_server::middleware::request_root_paths::normalize::normalize_root_paths;
use betterlite_server::middleware::request_root_paths::{RootPaths, get_request_root_path};
use serde::{Deserialize, Serialize};
use tower::ServiceExt as _;

use crate::helpers::HelperError;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    scalar_root: String,
    root_paths: Vec<String>,
    requests: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct Seen {
    root: String,
    path: String,
}

#[derive(Serialize)]
struct Output {
    normalized: Vec<String>,
    installed: bool,
    requests: Vec<Seen>,
}

fn failure<Message>(class: &str, message: Message) -> Box<HelperError>
where
    Message: std::fmt::Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}

async fn send(app: &Router, target: &str) -> Result<Seen, Box<HelperError>> {
    let request = Request::builder()
        .uri(target)
        .body(Body::empty())
        .map_err(|error| failure("InvalidRequest", error))?;
    let response = app
        .clone()
        .oneshot(request)
        .await
        .map_err(|error| failure("ServiceError", error))?;
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .map_err(|error| failure("ServiceError", error))?;
    serde_json::from_slice(&body).map_err(|error| failure("ServiceError", error))
}

async fn run(input: &Input) -> Result<Output, Box<HelperError>> {
    let normalized = normalize_root_paths(&input.root_paths);
    let settings = Settings {
        server_root_path: input.scalar_root.clone(),
        server_root_paths: input.root_paths.clone(),
    };
    let Some(paths) = RootPaths::from_settings(&settings) else {
        return Ok(Output {
            normalized,
            installed: false,
            requests: Vec::new(),
        });
    };
    let scalar_root = input.scalar_root.clone();
    let app = paths.wrap(Router::new().fallback(move |request: Request<Body>| {
        let seen = Seen {
            root: get_request_root_path(request.extensions(), &scalar_root),
            path: request.uri().path().to_owned(),
        };
        async move { serde_json::to_string(&seen).unwrap_or_default() }
    }));
    let mut requests = Vec::new();
    for target in &input.requests {
        requests.push(send(&app, target).await?);
    }
    Ok(Output {
        normalized,
        installed: true,
        requests,
    })
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|error| failure("RuntimeError", error))?;
        let output = runtime.block_on(run(input))?;
        serde_json::to_value(output).map_err(|error| failure("SerializeError", error))
    }
}
