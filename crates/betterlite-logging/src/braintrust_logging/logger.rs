use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::string::FromUtf8Error;
use std::sync::Mutex;

use betterlite_models::decisions::Json;
use http::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue, InvalidHeaderValue};
use secrecy::ExposeSecret as _;
use serde::Serialize;
use thiserror::Error;
use url::Url;

use crate::http_analytics_loggers::delivery::{DELIVERY_DEADLINE, Delivery, PostRequest};

use super::payload;
use super::payload::PayloadError;
use super::payload::{CallbackPath, SuccessEvent};
use super::payload::{metadata_text, object};
use super::settings::Settings;

const DEFAULT_PROJECT_NAME: &str = "litellm";

#[derive(Debug, Error)]
pub enum BraintrustError<E>
where
    E: StdError + 'static,
{
    #[error("the Braintrust request body could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("the Braintrust request failed")]
    Transport(#[source] E),
    #[error("Braintrust answered with status {status}: {body}")]
    Status { status: u16, body: String },
    #[error("the Braintrust response body is not UTF-8")]
    Body(#[source] FromUtf8Error),
    #[error("the Braintrust response body is not JSON")]
    Response(#[source] serde_json::Error),
    #[error("the Braintrust response has no string {field}")]
    ProjectField { field: &'static str },
    #[error("the Braintrust URL {url} is not valid")]
    Url {
        url: String,
        #[source]
        source: url::ParseError,
    },
    #[error("the Braintrust API base {url} cannot carry a path")]
    Base { url: String },
    #[error("the Braintrust Authorization header is not valid")]
    Header(#[source] InvalidHeaderValue),
    #[error("the Braintrust project cache is poisoned")]
    CachePoisoned,
    #[error(transparent)]
    Payload(#[from] PayloadError),
}

#[derive(Serialize)]
struct ProjectRequest<'name> {
    name: &'name str,
}

#[derive(Default)]
struct Projects {
    names: BTreeMap<String, String>,
    default_id: Option<String>,
}

pub struct BraintrustLogger<D> {
    settings: Settings,
    delivery: D,
    projects: Mutex<Projects>,
}

impl<D: Delivery> BraintrustLogger<D> {
    pub fn new(settings: Settings, delivery: D) -> Self {
        Self {
            settings,
            delivery,
            projects: Mutex::new(Projects::default()),
        }
    }

    pub async fn log_success_event(
        &self,
        event: &SuccessEvent<'_>,
        path: CallbackPath,
    ) -> Result<(), BraintrustError<D::Error>> {
        let project_id = self.project_id(event, path).await?;
        let request = payload::request_data(event, &self.settings.default_tags, path)?;
        let url = self.endpoint(&["project_logs", &project_id, "insert"])?;
        let body = payload::Events { events: [&request] };
        self.send(url, &body).await?;
        Ok(())
    }

    pub const fn log_failure_event(&self) {}

    fn endpoint(&self, segments: &[&str]) -> Result<Url, BraintrustError<D::Error>> {
        let base = &self.settings.api_base;
        let mut url = Url::parse(base).map_err(|source| BraintrustError::Url {
            url: base.clone(),
            source,
        })?;
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|()| BraintrustError::Base { url: base.clone() })?;
            path.pop_if_empty();
            path.extend(segments.iter().copied())
        };
        Ok(url)
    }

    async fn send<T>(&self, url: Url, body: &T) -> Result<Vec<u8>, BraintrustError<D::Error>>
    where
        T: Serialize + Sync + ?Sized,
    {
        let encoded = serde_json::to_vec(body).map_err(BraintrustError::Encode)?;
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let token = format!("Bearer {}", self.settings.api_key.expose_secret());
        let mut authorization = HeaderValue::from_str(&token).map_err(BraintrustError::Header)?;
        authorization.set_sensitive(true);
        headers.insert(AUTHORIZATION, authorization);
        let response = self
            .delivery
            .post(PostRequest {
                url,
                headers,
                body: encoded,
                deadline: DELIVERY_DEADLINE,
            })
            .await
            .map_err(BraintrustError::Transport)?;
        if response.status >= 400 {
            let text = String::from_utf8(response.body).map_err(BraintrustError::Body)?;
            return Err(BraintrustError::Status {
                status: response.status,
                body: text,
            });
        }
        Ok(response.body)
    }

    async fn project_id(
        &self,
        event: &SuccessEvent<'_>,
        path: CallbackPath,
    ) -> Result<String, BraintrustError<D::Error>> {
        let metadata = event
            .metadata
            .filter(|value| !matches!(value, Json::Null))
            .map(|value| object(value, "litellm_params.metadata"))
            .transpose()?;
        let Some(fields) = metadata else {
            return self.default_project().await;
        };
        if let Some(id) = metadata_text(Some(fields), "project_id")? {
            return Ok(id.to_owned());
        }
        match metadata_text(Some(fields), "project_name")? {
            Some(name) if !name.is_empty() => self.resolve_project(name, path).await,
            Some(_) | None => self.default_project().await,
        }
    }

    pub async fn resolve_project(
        &self,
        name: &str,
        path: CallbackPath,
    ) -> Result<String, BraintrustError<D::Error>> {
        if let Some(id) = self.with_projects(|projects| projects.names.get(name).cloned())? {
            return Ok(id);
        }
        let route: &[&str] = match path {
            CallbackPath::Sync => &["project"],
            CallbackPath::Async => &["project", "register"],
        };
        let id = self.request_project(self.endpoint(route)?, name).await?;
        self.with_projects(|projects| {
            projects.names.insert(name.to_owned(), id.clone());
        })?;
        Ok(id)
    }

    async fn default_project(&self) -> Result<String, BraintrustError<D::Error>> {
        let Some(id) = self.with_projects(|projects| projects.default_id.clone())? else {
            return self.create_default_project().await;
        };
        Ok(id)
    }

    async fn create_default_project(&self) -> Result<String, BraintrustError<D::Error>> {
        let url = self.endpoint(&["project"])?;
        let id = self.request_project(url, DEFAULT_PROJECT_NAME).await?;
        self.with_projects(|projects| {
            projects.default_id = Some(id.clone());
        })?;
        Ok(id)
    }

    async fn request_project(
        &self,
        url: Url,
        name: &str,
    ) -> Result<String, BraintrustError<D::Error>> {
        let response = self.send(url, &ProjectRequest { name }).await?;
        let parsed =
            serde_json::from_slice::<Json>(&response).map_err(BraintrustError::Response)?;
        if let Json::Object(fields) = &parsed
            && let Some(Json::Text(id)) = fields.get("id")
        {
            return Ok(id.clone());
        }
        Err(BraintrustError::ProjectField { field: "id" })
    }

    fn with_projects<R, F>(&self, change: F) -> Result<R, BraintrustError<D::Error>>
    where
        F: FnOnce(&mut Projects) -> R,
    {
        let mut projects = self
            .projects
            .lock()
            .map_err(|_poisoned| BraintrustError::CachePoisoned)?;
        Ok(change(&mut projects))
    }
}
