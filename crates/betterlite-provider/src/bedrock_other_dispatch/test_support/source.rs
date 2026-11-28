use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::CredentialSource;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::transcription::auth::ChainCredentials;
use betterlite_config::sections::bedrock_settings::AwsSettings;

// The credential lookup B595 would run; the fixture answers with the one identity it was built with
#[derive(Clone)]
pub struct Source {
    credentials: Credentials,
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
}

impl CredentialSource for Source {
    fn resolve(
        &self,
        settings: AwsSettings,
        region: String,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send {
        self.record(region);
        // a deployment's own static keys sign the request, as the AWS resolver does
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
        self.record(settings.region_name.clone().unwrap_or_default());
        ready(Ok::<Credentials, CredentialError>(self.credentials.clone()))
    }
}
