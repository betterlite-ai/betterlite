mod script;

use std::future::Future;

use axum::http::Method;
use axum::http::header::{ACCEPT, AUTHORIZATION};
use betterlite_roi::settings::ROISettings;
use betterlite_server::roi_github::client::GitHub;
use betterlite_server::roi_github::error::SourceError;
use betterlite_server::roi_github::models::{PAGINATION_ERROR, RepositoryItem};
use betterlite_server::roi_github::{pages, profile_email, repositories};
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

impl Input {
    fn body_bytes(&self) -> Result<Option<Vec<u8>>, Box<HelperError>> {
        self.json_body
            .as_ref()
            .map(serde_json::to_vec)
            .transpose()
            .map_err(|error| helper_error("ValueError", error.to_string()))
    }
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
        .map_err(|error| helper_error("ValueError", error.to_string()))
}

async fn run(input: &Input) -> HelperResult {
    let settings = settings(input)?;
    let script = Script::new(input.replies.clone());
    let github = GitHub::new(&settings, script.clone())
        .map_err(|error| helper_error("SourceError", error.to_string()))?;
    let payload = match input.operation.as_str() {
        "init" => init(&github),
        "request" => request(&github, input).await?,
        "fetch_page" => {
            fetch_page(
                &github,
                &input.path,
                &input.params,
                input.page,
                input.error_message.as_deref(),
            )
            .await?
        }
        "pages" => pages(&github, &input.path, &input.params, input.limit).await?,
        "repositories" => repositories(&github, input).await?,
        "test_repositories" => test_repositories(&github, input).await?,
        "profile_email" => profile_email(&github, input).await?,
        other => {
            return Err(helper_error(
                "ValueError",
                format!("unsupported operation {other}"),
            ));
        }
    };
    Ok(with_requests(payload, &script))
}

fn with_requests(payload: Value, script: &Script) -> Value {
    let Value::Object(mut fields) = payload else {
        return payload;
    };
    fields.insert("requests".to_owned(), Value::from(script.seen()));
    Value::Object(fields)
}

fn settings(input: &Input) -> Result<ROISettings, Box<HelperError>> {
    serde_json::from_value(json!({
        "github_api_url": input.api_url,
        "github_token": input.token,
    }))
    .map_err(|error| helper_error("ValueError", error.to_string()))
}

fn init(github: &GitHub<Script>) -> Value {
    json!({
        "api_url": github.api_url,
        "timeout_seconds": github.timeout.as_secs(),
        "accept": github.headers.get(ACCEPT).and_then(|value| value.to_str().ok()),
        "authorization": github.headers.get(AUTHORIZATION).and_then(|value| value.to_str().ok()),
    })
}

async fn request(github: &GitHub<Script>, input: &Input) -> HelperResult {
    let method = Method::from_bytes(input.method.as_bytes())
        .map_err(|error| helper_error("ValueError", error.to_string()))?;
    let response = github
        .request(method, &input.path, &input.params, input.body_bytes()?)
        .await
        .map_err(|error| helper_error("SourceError", error.to_string()))?;
    Ok(json!({"status": response.status}))
}

async fn fetch_page(
    github: &GitHub<Script>,
    path: &str,
    params: &[(String, String)],
    page: i64,
    error_message: Option<&str>,
) -> HelperResult {
    let message = error_message.unwrap_or(PAGINATION_ERROR);
    let fetch = pages::fetch_page::<RepositoryItem, _>(github, path, params, page, message);
    let (items, has_next) = github_result(fetch).await?;
    Ok(json!({"items": items, "has_next": has_next}))
}

async fn pages(
    github: &GitHub<Script>,
    path: &str,
    params: &[(String, String)],
    limit: u32,
) -> HelperResult {
    let stream = pages::pages::<RepositoryItem, _>(github, path, params, limit);
    let collected = github_result(stream.try_collect::<Vec<Vec<RepositoryItem>>>()).await?;
    Ok(json!({"pages": collected}))
}

async fn repositories(github: &GitHub<Script>, input: &Input) -> HelperResult {
    let (repositories, has_more) =
        github_result(repositories::repositories(github, &input.query, input.page)).await?;
    Ok(json!({"repositories": repositories, "has_more": has_more}))
}

async fn test_repositories(github: &GitHub<Script>, input: &Input) -> HelperResult {
    github_result(repositories::test_repositories(github, &input.repos)).await?;
    Ok(json!({}))
}

async fn profile_email(github: &GitHub<Script>, input: &Input) -> HelperResult {
    let mut emails = Vec::with_capacity(input.calls.len());
    for call in &input.calls {
        emails.push(profile_email::profile_email(github, &call.login, &call.fallback).await);
    }
    Ok(json!({"emails": emails}))
}

async fn github_result<F, T>(future: F) -> Result<T, Box<HelperError>>
where
    F: Future<Output = Result<T, SourceError>>,
{
    match future.await {
        Ok(value) => Ok(value),
        Err(error) => Err(helper_error("SourceError", error.to_string())),
    }
}

fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
