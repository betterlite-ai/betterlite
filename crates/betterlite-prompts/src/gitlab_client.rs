use auth::RequestHeader;
use config::{DEFAULT_BASE_URL, GitLabConfig};
use error::{FetchFailure, GitLabClientError};
use transport::{GitLabResponse, GitLabTransport};
use url::FileEndpoint;

pub mod auth;
pub mod config;
pub mod error;
mod fetch;
mod init;
pub mod payload;
pub(crate) mod tests;
pub mod transport;
mod url;

pub struct GitLabClient<T> {
    project: String,
    ref_name: String,
    base_url: String,
    headers: Vec<RequestHeader>,
    transport: T,
}

impl<T: GitLabTransport> GitLabClient<T> {
    pub fn new(config: &GitLabConfig, transport: T) -> Result<Self, GitLabClientError> {
        let (project, access_token) = init::require_credentials(config)?;
        let auth_method = config.auth_method.as_deref().unwrap_or("token");
        Ok(Self {
            project,
            ref_name: config.default_ref(),
            base_url: config
                .base_url
                .clone()
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_owned()),
            headers: auth::request_headers(auth_method, access_token),
            transport,
        })
    }

    pub fn set_ref(&mut self, ref_name: &str) -> Result<(), GitLabClientError> {
        if ref_name.is_empty() {
            return Err(GitLabClientError::EmptyRef);
        }
        ref_name.clone_into(&mut self.ref_name);
        Ok(())
    }

    // An empty ref_override counts as absent, like Python's `ref or self.ref`.
    pub async fn get_file_content(
        &self,
        file_path: &str,
        ref_override: Option<&str>,
    ) -> Result<Option<String>, GitLabClientError> {
        let ref_name = ref_override
            .filter(|name| !name.is_empty())
            .unwrap_or(&self.ref_name);
        let raw = self.request(FileEndpoint::Raw, file_path, ref_name).await?;
        if raw.status != 404 {
            return Ok(Some(payload::decode_text(&raw.body)));
        }
        let json = self
            .request(FileEndpoint::Json, file_path, ref_name)
            .await?;
        if json.status == 404 {
            return Ok(None);
        }
        payload::decode_file_payload(&json.body).map_err(|source| {
            fetch::classify(
                FileEndpoint::Json,
                &self.project,
                file_path,
                FetchFailure::Payload(source),
            )
        })
    }

    // A 404 comes back as a response so the raw path can fall back to the JSON endpoint.
    async fn request(
        &self,
        endpoint: FileEndpoint,
        file_path: &str,
        ref_name: &str,
    ) -> Result<GitLabResponse, GitLabClientError> {
        let url = url::file_url(&self.base_url, &self.project, file_path, ref_name, endpoint);
        let response = self
            .transport
            .get(&url, &self.headers, fetch::MAX_FILE_BYTES)
            .await
            .map_err(|source| {
                fetch::classify(
                    endpoint,
                    &self.project,
                    file_path,
                    FetchFailure::Transport(source),
                )
            })?;
        if response.status != 404 && !(200..300).contains(&response.status) {
            return Err(fetch::classify(
                endpoint,
                &self.project,
                file_path,
                FetchFailure::Status(response.status),
            ));
        }
        Ok(response)
    }
}
