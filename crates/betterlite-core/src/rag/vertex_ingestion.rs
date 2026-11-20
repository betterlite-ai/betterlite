pub mod error;
pub mod ports;

mod corpus;
mod json;
mod upload;

mod tests;

use async_trait::async_trait;
use betterlite_config::rag::document::ingest::VertexAIVectorStoreOptions;
use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use secrecy::{ExposeSecret as _, SecretString};

use crate::api::vertex_location::validate_vertex_location;
use crate::rag::ingestion::IngestionState;
use crate::rag::ingestion::store::{RagStore, StoreRequest, StoreResult};

use error::VertexIngestionError;
use ports::VertexIngestionPorts;

const API_VERSION: &str = "v1beta1";

pub const DEFAULT_LOCATION: &str = "us-central1";

pub const DEFAULT_CORPUS_NAME: &str = "litellm-rag-corpus";

pub struct VertexIngestion {
    state: IngestionState,
    options: VertexAIVectorStoreOptions,
    project: Option<String>,
    location: String,
    credentials: CredentialChoice,
    ports: VertexIngestionPorts,
}

enum CredentialChoice {
    None,
    Record(SecretString),
    Settings(ConfiguredCredentials),
}

impl VertexIngestion {
    pub const SUPPORTS_EXISTING_FILE_ID: bool = IngestionState::SUPPORTS_EXISTING_FILE_ID;

    #[must_use]
    pub fn new(
        state: IngestionState,
        options: VertexAIVectorStoreOptions,
        settings: &VertexSettings,
        ports: VertexIngestionPorts,
    ) -> Self {
        let project = resolved_project(&options, settings);
        let location = resolved_location(&options, settings);
        let credentials = credential_choice(&options, settings);
        Self {
            state,
            options,
            project,
            location,
            credentials,
            ports,
        }
    }

    pub fn embed(&self, _chunks: &[String]) -> Option<Vec<Vec<f64>>> {
        // Vertex AI embeds when the file joins the corpus, so this step has no work
        None
    }

    fn display_name(&self) -> &str {
        self.state
            .ingest_name
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or(DEFAULT_CORPUS_NAME)
    }

    fn existing_corpus_id(&self) -> Option<String> {
        self.options
            .vector_store_id
            .as_deref()
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    }

    async fn store_file(
        &self,
        corpus_id: &str,
        request: &StoreRequest<'_>,
    ) -> Result<Option<String>, VertexIngestionError> {
        let Some(content) = request.file_content.filter(|content| !content.is_empty()) else {
            return Ok(None);
        };
        let Some(filename) = request.filename.filter(|filename| !filename.is_empty()) else {
            return Ok(None);
        };
        if corpus_id.is_empty() {
            return Ok(None);
        }
        let file_id =
            upload::upload_file(self, corpus_id, filename, content, request.content_type).await?;
        Ok(Some(file_id))
    }

    async fn bearer_token(
        &self,
        project: Option<&str>,
    ) -> Result<SecretString, VertexIngestionError> {
        let credentials = self.credentials()?;
        self.ports
            .token
            .access_token(credentials.as_ref(), project)
            .await
            .map(|(token, _project)| token)
            .map_err(VertexIngestionError::from)
    }

    fn credentials(&self) -> Result<Option<ConfiguredCredentials>, VertexIngestionError> {
        match &self.credentials {
            CredentialChoice::None => Ok(None),
            CredentialChoice::Record(text) => {
                ConfiguredCredentials::from_text(text.expose_secret().to_owned())
                    .map(Some)
                    .map_err(|source| VertexIngestionError::Credentials { source })
            }
            CredentialChoice::Settings(credentials) => Ok(Some(credentials.clone())),
        }
    }

    fn base_url(&self) -> Result<String, VertexIngestionError> {
        let location = validate_vertex_location(Some(self.location.as_str()))?;
        Ok(match location {
            "global" => "https://aiplatform.googleapis.com".to_owned(),
            regional if regional.contains('-') => {
                format!("https://{regional}-aiplatform.googleapis.com")
            }
            geography => format!("https://aiplatform.{geography}.rep.googleapis.com"),
        })
    }
}

#[async_trait]
impl RagStore for VertexIngestion {
    type Error = VertexIngestionError;

    async fn store(&self, request: StoreRequest<'_>) -> Result<StoreResult, Self::Error> {
        let Some(project) = self.project.as_deref().filter(|value| !value.is_empty()) else {
            return Err(VertexIngestionError::MissingProject);
        };
        let corpus_id = match self.existing_corpus_id() {
            Some(corpus_id) => corpus_id,
            None => corpus::create_corpus(self, project, self.display_name()).await?,
        };
        let file_id = self.store_file(&corpus_id, &request).await?;
        Ok((Some(corpus_id), file_id))
    }
}

fn resolved_project(
    options: &VertexAIVectorStoreOptions,
    settings: &VertexSettings,
) -> Option<String> {
    options
        .vertex_project
        .as_deref()
        .or(options.vertex_ai_project.as_deref())
        .or(settings.project.as_deref())
        .map(str::to_owned)
}

fn resolved_location(options: &VertexAIVectorStoreOptions, settings: &VertexSettings) -> String {
    let candidates = [
        options.vertex_location.as_deref(),
        options.vertex_ai_location.as_deref(),
        settings.location.as_deref(),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|location| !location.is_empty())
        .unwrap_or(DEFAULT_LOCATION)
        .to_owned()
}

fn credential_choice(
    options: &VertexAIVectorStoreOptions,
    settings: &VertexSettings,
) -> CredentialChoice {
    if let Some(text) = options
        .vertex_credentials
        .as_ref()
        .or(options.vertex_ai_credentials.as_ref())
    {
        return CredentialChoice::Record(text.clone());
    }
    settings.credentials.as_ref().map_or_else(
        || CredentialChoice::None,
        |credentials| CredentialChoice::Settings(credentials.clone()),
    )
}
