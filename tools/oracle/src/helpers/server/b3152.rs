mod script;

use std::future::Future;

use axum::http::Method;
use axum::http::header::{ACCEPT, AUTHORIZATION};
use betterlite_roi::settings::ROISettings;
use betterlite_server::roi_github::client::{FOLLOW_REDIRECTS, GitHub};
use betterlite_server::roi_github::error::SourceError;
use betterlite_server::roi_github::models::{PAGINATION_ERROR, RepositoryItem};
use futures_util::TryStreamExt as _;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};
use script::{ReplyInput, Script};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    pub operation: String,
    #[serde(default = "default_api_url")]
    pub api_url: String,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub method: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub params: Vec<(String, String)>,
    #[serde(default)]
    pub json_body: Option<Value>,
    #[serde(default)]
    pub page: i64,
    #[serde(default)]
    pub limit: u32,
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub repos: Vec<String>,
    #[serde(default)]
    pub calls: Vec<ProfileCall>,
    #[serde(default)]
    pub error_message: Option<String>,
    #[serde(default)]
    pub replies: Vec<ReplyInput>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ProfileCall {
    pub login: String,
    #[serde(default)]
    pub fallback: String,
}

fn default_api_url() -> String {
    "https://api.github.com".to_owned()
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        block_on(run(input))?
    }
}

fn block_on<F>(future: F) -> Result<F::Output, Box<HelperError>>
where
    F: Future,
{
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .map(|runtime| runtime.block_on(future))
        .map_err(|error| input_error(&error.to_string()))
}

async fn run(input: &Input) -> HelperResult {
    let settings = settings(input)?;
    let script = Script::new(input.replies.clone());
    let github = GitHub::new(&settings, script.clone()).map_err(|error| source_error(&error))?;
    match input.operation.as_str() {
        "init" => Ok(init(&github)),
        "request" => request(&github, input, &script).await,
        "fetch_page" => fetch_page(&github, input, &script).await,
        "pages" => pages(&github, input, &script).await,
        "repositories" => repositories(&github, input, &script).await,
        "test_repositories" => test_repositories(&github, input, &script).await,
        "profile_email" => profile_email(&github, input, &script).await,
        other => Err(input_error(&format!("unsupported operation {other}"))),
    }
}

fn settings(input: &Input) -> Result<ROISettings, Box<HelperError>> {
    serde_json::from_value(json!({
        "github_api_url": input.api_url,
        "github_token": input.token,
    }))
    .map_err(|error| input_error(&error.to_string()))
}

fn init(github: &GitHub<Script>) -> Value {
    json!({
        "api_url": github.api_url,
        "timeout_seconds": github.timeout.as_secs(),
        "follow_redirects": FOLLOW_REDIRECTS,
        "accept": github.headers.get(ACCEPT).and_then(|value| value.to_str().ok()),
        "authorization": github.headers.get(AUTHORIZATION).and_then(|value| value.to_str().ok()),
    })
}

async fn request(github: &GitHub<Script>, input: &Input, script: &Script) -> HelperResult {
    let body = input
        .json_body
        .as_ref()
        .map(serde_json::to_vec)
        .transpose()
        .map_err(|error| input_error(&error.to_string()))?;
    let response = github
        .request(
            Method::from_bytes(input.method.as_bytes())
                .map_err(|error| input_error(&error.to_string()))?,
            &input.path,
            &input.params,
            body,
        )
        .await
        .map_err(|error| source_error(&error))?;
    Ok(json!({"status": response.status, "requests": script.seen()}))
}

async fn fetch_page(github: &GitHub<Script>, input: &Input, script: &Script) -> HelperResult {
    let message = input
        .error_message
        .clone()
        .unwrap_or_else(|| PAGINATION_ERROR.to_owned());
    let (items, has_next) = github
        .fetch_page::<RepositoryItem>(&input.path, &input.params, input.page, &message)
        .await
        .map_err(|error| source_error(&error))?;
    Ok(json!({"items": items, "has_next": has_next, "requests": script.seen()}))
}

async fn pages(github: &GitHub<Script>, input: &Input, script: &Script) -> HelperResult {
    let collected = github
        .pages::<RepositoryItem>(&input.path, &input.params, input.limit)
        .try_collect::<Vec<Vec<RepositoryItem>>>()
        .await
        .map_err(|error| source_error(&error))?;
    Ok(json!({"pages": collected, "requests": script.seen()}))
}

async fn repositories(github: &GitHub<Script>, input: &Input, script: &Script) -> HelperResult {
    let (repositories, has_more) = github
        .repositories(&input.query, input.page)
        .await
        .map_err(|error| source_error(&error))?;
    Ok(json!({
        "repositories": repositories,
        "has_more": has_more,
        "requests": script.seen(),
    }))
}

async fn test_repositories(
    github: &GitHub<Script>,
    input: &Input,
    script: &Script,
) -> HelperResult {
    github
        .test_repositories(&input.repos)
        .await
        .map_err(|error| source_error(&error))?;
    Ok(json!({"requests": script.seen()}))
}

async fn profile_email(github: &GitHub<Script>, input: &Input, script: &Script) -> HelperResult {
    let mut emails = Vec::with_capacity(input.calls.len());
    for call in &input.calls {
        emails.push(github.profile_email(&call.login, &call.fallback).await);
    }
    Ok(json!({"emails": emails, "requests": script.seen()}))
}

fn source_error(error: &SourceError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "SourceError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn input_error(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
