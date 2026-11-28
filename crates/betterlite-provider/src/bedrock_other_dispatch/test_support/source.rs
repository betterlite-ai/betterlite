use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::count_call::handler::CountTokensCredentials;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::embedding_call::CredentialSource as EmbeddingCredentials;
use betterlite_bedrock::image_call::plan::CredentialSource as ImageCredentials;
use betterlite_bedrock::rerank::plan::CredentialSource as RerankCredentials;
use betterlite_bedrock::transcription::auth::ChainCredentials;
use betterlite_config::sections::bedrock_settings::AwsSettings;

// The credential lookup B595 would run; the fixture answers with the one identity it was built with
#[derive(Clone)]
pub struct Source {
    pub credentials: Credentials,
    regions: Arc<Mutex<Vec<String>>>,
}

impl Source {
    #[must_use]
    pub fn new(credentials: Credentials) -> Self {
        Self {
            credentials,
            regions: Arc::new(Mutex::new(Vec::new())),
        }
    }

    #[must_use]
    pub fn regions(&self) -> Vec<String> {
        self.regions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, region: String) {
        self.regions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(region);
    }

    // Every leaf but the token counter ignores its settings and gets the recorded identity back
    fn resolved(
        &self,
        region: String,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send {
        self.record(region);
        ready(Ok(self.credentials.clone()))
    }
}

impl EmbeddingCredentials for Source {
    fn resolve(
        &self,
        _settings: AwsSettings,
        region: String,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send {
        self.resolved(region)
    }
}

impl ImageCredentials for Source {
    fn resolve(
        &self,
        _settings: AwsSettings,
        region: String,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send {
        self.resolved(region)
    }
}

impl RerankCredentials for Source {
    fn resolve(
        &self,
        _settings: AwsSettings,
        region: String,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send {
        self.resolved(region)
    }
}

impl CountTokensCredentials for Source {
    fn resolve(
        &self,
        settings: AwsSettings,
        region: String,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send {
        self.record(region);
        // B594's static-key branch hands the deployment's own keys straight back
        let credentials = match (settings.access_key_id, settings.secret_access_key) {
            (Some(access_key_id), Some(secret_access_key)) => Credentials {
                access_key_id,
                secret_access_key,
                session_token: settings.session_token,
            },
            _ => self.credentials.clone(),
        };
        ready(Ok::<Credentials, CredentialError>(credentials))
    }
}

impl ChainCredentials for Source {
    fn resolve(
        &self,
        settings: &AwsSettings,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send {
        self.resolved(settings.region_name.clone().unwrap_or_default())
    }
}
