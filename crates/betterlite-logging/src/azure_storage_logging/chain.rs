use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use http::Method;
use secrecy::SecretString;
use serde::Deserialize;
use tokio::runtime::Handle;

use super::token::{
    ChainError, ChainProviderFactory, ChainTokenProvider, DEFAULT_AUTHORITY_HOST, STORAGE_SCOPE,
    TOKEN_PATH,
};
use super::transport::{STORAGE_TIMEOUT, StorageRequest, StorageTransport};

const IMDS_URL: &str = "http://169.254.169.254/metadata/identity/oauth2/token";
const IMDS_API_VERSION: &str = "2018-02-01";
const CLIENT_ASSERTION_TYPE: &str = "urn:ietf:params:oauth:client-assertion-type:jwt-bearer";
const FIRST_ERROR_STATUS: u16 = 400;
const BAD_REQUEST: u16 = 400;

#[derive(Debug, Deserialize)]
struct AccessToken {
    access_token: Option<SecretString>,
}

fn text(environment: &dyn EnvironmentLookup, name: &str) -> Option<String> {
    environment.lookup(name).filter(|value| !value.is_empty())
}

fn parse(body: &[u8]) -> Option<SecretString> {
    serde_json::from_slice::<AccessToken>(body)
        .ok()
        .and_then(|token| token.access_token)
}

fn workload_request(tenant_id: &str, client_id: &str, assertion: &str) -> StorageRequest {
    let form = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("grant_type", "client_credentials")
        .append_pair("client_id", client_id)
        .append_pair("scope", STORAGE_SCOPE)
        .append_pair("client_assertion_type", CLIENT_ASSERTION_TYPE)
        .append_pair("client_assertion", assertion)
        .finish();
    StorageRequest {
        method: Method::POST,
        url: format!("{DEFAULT_AUTHORITY_HOST}/{tenant_id}{TOKEN_PATH}"),
        query: Vec::new(),
        headers: vec![(
            "content-type".to_owned(),
            "application/x-www-form-urlencoded".to_owned(),
        )],
        body: form.into_bytes(),
        deadline: STORAGE_TIMEOUT,
    }
}

fn managed_identity_request(client_id: Option<&str>) -> StorageRequest {
    let resource = STORAGE_SCOPE.trim_end_matches("/.default");
    let mut query = vec![
        ("api-version".to_owned(), IMDS_API_VERSION.to_owned()),
        ("resource".to_owned(), resource.to_owned()),
    ];
    if let Some(client_id) = client_id {
        query.push(("client_id".to_owned(), client_id.to_owned()));
    }
    StorageRequest {
        method: Method::GET,
        url: IMDS_URL.to_owned(),
        query,
        headers: vec![("Metadata".to_owned(), "true".to_owned())],
        body: Vec::new(),
        deadline: STORAGE_TIMEOUT,
    }
}

// LiteLLM builds a DeploymentIdentityCredential chain: workload identity, then user-assigned, then system
pub struct DeploymentIdentityChain {
    pub transport: Arc<dyn StorageTransport>,
    pub handle: Handle,
    pub tenant_id: Option<String>,
    pub client_id: Option<String>,
    pub federated_token_file: Option<String>,
}

impl DeploymentIdentityChain {
    fn workload(&self) -> Option<(String, String, String)> {
        let tenant_id = self.tenant_id.clone()?;
        let client_id = self.client_id.clone()?;
        let token_file = self.federated_token_file.clone()?;
        Some((tenant_id, client_id, token_file))
    }

    fn run(&self, request: StorageRequest) -> Result<SecretString, ChainError> {
        let response = self
            .handle
            .block_on(self.transport.send(request))
            .map_err(|_error| ChainError::Unavailable)?;
        if response.status == BAD_REQUEST {
            return Err(ChainError::Unavailable);
        }
        if response.status >= FIRST_ERROR_STATUS {
            return Err(ChainError::Authentication {
                message: format!("IMDS answered with status {}", response.status),
            });
        }
        parse(&response.body).ok_or(ChainError::Unavailable)
    }
}

impl ChainTokenProvider for DeploymentIdentityChain {
    fn token(&self) -> Result<SecretString, ChainError> {
        if let Some((tenant_id, client_id, token_file)) = self.workload() {
            let assertion = std::fs::read_to_string(&token_file).map_err(|source| {
                ChainError::Authentication {
                    message: source.to_string(),
                }
            })?;
            match self.run(workload_request(&tenant_id, &client_id, &assertion)) {
                Ok(token) => return Ok(token),
                Err(ChainError::Unavailable) => {}
                Err(error) => return Err(error),
            }
        }
        if let Some(client_id) = self.client_id.as_deref() {
            match self.run(managed_identity_request(Some(client_id))) {
                Ok(token) => return Ok(token),
                Err(ChainError::Unavailable) => {}
                Err(error) => return Err(error),
            }
        }
        self.run(managed_identity_request(None))
    }
}

pub struct DeploymentIdentityFactory {
    pub transport: Arc<dyn StorageTransport>,
    pub environment: Arc<dyn EnvironmentLookup + Send + Sync>,
}

impl ChainProviderFactory for DeploymentIdentityFactory {
    fn provider(&self) -> Result<Arc<dyn ChainTokenProvider>, ChainError> {
        let handle = Handle::try_current().map_err(|_error| ChainError::Unavailable)?;
        Ok(Arc::new(DeploymentIdentityChain {
            transport: Arc::clone(&self.transport),
            handle,
            tenant_id: text(self.environment.as_ref(), "AZURE_TENANT_ID"),
            client_id: text(self.environment.as_ref(), "AZURE_CLIENT_ID"),
            federated_token_file: text(self.environment.as_ref(), "AZURE_FEDERATED_TOKEN_FILE"),
        }))
    }
}
