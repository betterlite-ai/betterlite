use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_models::decisions::Json;
use http::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use url::Url;

use crate::http_analytics_loggers::delivery::{DELIVERY_DEADLINE, Delivery, PostRequest};

use super::error::BraintrustError;
use super::event::{CallbackPath, SuccessEvent};
use super::payload;
use super::payload::{metadata_text, optional_object};
use super::settings::Settings;

const DEFAULT_PROJECT_NAME: &str = "litellm";

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

    // The path picks LiteLLM's project endpoint and its time-to-first-token rule.
    pub async fn log_success_event(
        &self,
        event: &SuccessEvent<'_>,
        path: CallbackPath,
    ) -> Result<(), BraintrustError<D::Error>> {
        let project_id = self.project_id(event, path).await?;
        let request = payload::request_data(event, &self.settings.default_tags, path)?;
        self.insert(&project_id, request).await
    }

    // LiteLLM's failure callback delegates to CustomLogger's no-op, so a failure event sends nothing
    pub const fn log_failure_event(&self) {}

    async fn insert(&self, project_id: &str, event: Json) -> Result<(), BraintrustError<D::Error>> {
        let url = format!(
            "{}/project_logs/{project_id}/insert",
            self.settings.api_base
        );
        let body = Json::Object(IndexMap::from([(
            "events".to_owned(),
            Json::List(vec![event]),
        )]));
        let _response = self.send(&url, &body).await?;
        Ok(())
    }

    async fn send(&self, url: &str, body: &Json) -> Result<Vec<u8>, BraintrustError<D::Error>> {
        let encoded = serde_json::to_vec(body).map_err(BraintrustError::Encode)?;
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let authorization = format!("Bearer {}", self.settings.api_key.expose_secret());
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&authorization).map_err(BraintrustError::Header)?,
        );
        let target = Url::parse(url).map_err(|source| BraintrustError::Url {
            url: url.to_owned(),
            source,
        })?;
        let response = self
            .delivery
            .post(PostRequest {
                url: target,
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
        let Some(fields) = optional_object(event.metadata, "litellm_params.metadata")? else {
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
        if let Some(id) = self.cached(name)? {
            return Ok(id);
        }
        let endpoint = match path {
            CallbackPath::Sync => "/project",
            CallbackPath::Async => "/project/register",
        };
        let url = format!("{}{endpoint}", self.settings.api_base);
        let id = self.request_project(&url, name).await?;
        self.remember(name, &id)?;
        Ok(id)
    }

    async fn default_project(&self) -> Result<String, BraintrustError<D::Error>> {
        if let Some(id) = self.default_cached()? {
            return Ok(id);
        }
        let url = format!("{}/project", self.settings.api_base);
        let id = self.request_project(&url, DEFAULT_PROJECT_NAME).await?;
        self.remember_default(&id)?;
        Ok(id)
    }

    async fn request_project(
        &self,
        url: &str,
        name: &str,
    ) -> Result<String, BraintrustError<D::Error>> {
        let body = Json::Object(IndexMap::from([(
            "name".to_owned(),
            Json::Text(name.to_owned()),
        )]));
        let response = self.send(url, &body).await?;
        let parsed =
            serde_json::from_slice::<Json>(&response).map_err(BraintrustError::Response)?;
        if let Json::Object(fields) = &parsed
            && let Some(Json::Text(id)) = fields.get("id")
        {
            return Ok(id.clone());
        }
        Err(BraintrustError::ProjectField { field: "id" })
    }

    fn cached(&self, name: &str) -> Result<Option<String>, BraintrustError<D::Error>> {
        let projects = self
            .projects
            .lock()
            .map_err(|_poisoned| BraintrustError::CachePoisoned)?;
        Ok(projects.names.get(name).cloned())
    }

    fn remember(&self, name: &str, id: &str) -> Result<(), BraintrustError<D::Error>> {
        self.projects
            .lock()
            .map_err(|_poisoned| BraintrustError::CachePoisoned)?
            .names
            .insert(name.to_owned(), id.to_owned());
        Ok(())
    }

    fn default_cached(&self) -> Result<Option<String>, BraintrustError<D::Error>> {
        let projects = self
            .projects
            .lock()
            .map_err(|_poisoned| BraintrustError::CachePoisoned)?;
        Ok(projects.default_id.clone())
    }

    fn remember_default(&self, id: &str) -> Result<(), BraintrustError<D::Error>> {
        self.projects
            .lock()
            .map_err(|_poisoned| BraintrustError::CachePoisoned)?
            .default_id = Some(id.to_owned());
        Ok(())
    }
}
