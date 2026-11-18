use std::future::{Future, ready};
use std::sync::atomic::{AtomicU64, Ordering};

use betterlite_bedrock::agentcore_stream::IdSource;
use betterlite_bedrock::aws_settings::AuthError;
use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::realtime_events::stream::StreamError;
use betterlite_bedrock::realtime_transport::error::ProviderClass;
use betterlite_bedrock::realtime_transport::prepare::RealtimeCredentials;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use secrecy::SecretString;
use thiserror::Error;
use uuid::Uuid;

use super::{Auth, CredentialMode};

#[derive(Debug, Clone, Error)]
#[error("{message}")]
pub(super) struct FakeError {
    class_name: String,
    message: String,
}

impl FakeError {
    fn new(class_name: &str, message: &str) -> Self {
        Self {
            class_name: class_name.to_owned(),
            message: message.to_owned(),
        }
    }
}

impl ProviderClass for FakeError {
    fn provider_class_name(&self) -> Option<&str> {
        Some(&self.class_name)
    }

    fn provider_message(&self) -> Option<&str> {
        Some(&self.message)
    }
}

pub(super) fn stream_error(message: &str, class_name: &str) -> StreamError<FakeError> {
    StreamError {
        message: message.to_owned(),
        source: Some(FakeError::new(class_name, message)),
    }
}

pub(super) struct FakeCredentials {
    pub(super) mode: CredentialMode,
    pub(super) auth: Option<Auth>,
}

impl FakeCredentials {
    // An empty identity is what boto3's frozen credentials report when nothing was configured
    fn resolved(&self) -> Credentials {
        let auth = self.auth.clone();
        Credentials {
            access_key_id: SecretString::from(
                auth.as_ref()
                    .map_or(String::new(), |value| value.access_key_id.clone()),
            ),
            secret_access_key: SecretString::from(
                auth.as_ref()
                    .map_or(String::new(), |value| value.secret_access_key.clone()),
            ),
            session_token: auth
                .and_then(|value| value.session_token)
                .map(SecretString::from),
        }
    }
}

impl RealtimeCredentials for FakeCredentials {
    fn resolve(
        &self,
        _settings: &AwsSettings,
        _region: &str,
    ) -> impl Future<Output = Result<Option<Credentials>, CredentialError>> + Send {
        ready(match self.mode {
            CredentialMode::Resolved => Ok(Some(self.resolved())),
            CredentialMode::Absent => Ok(None),
            CredentialMode::Error => Err(CredentialError::Auth(AuthError::SessionTags)),
        })
    }
}

pub(super) struct FixedIds(pub(super) AtomicU64);

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        Uuid::from_u128(u128::from(self.0.fetch_add(1, Ordering::SeqCst)))
    }
}
