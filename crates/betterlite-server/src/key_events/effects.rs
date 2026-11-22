use async_trait::async_trait;

use crate::key_event_sinks::email::{CreatedKey, EmailError, LegacyEmail};
use crate::key_event_sinks::secrets::SecretSink;

#[derive(Debug, thiserror::Error)]
pub enum KeyEmailError {
    #[error(transparent)]
    Rejected(#[from] EmailError),
}

// the key hooks need the legacy email's decision as a Result so a failure becomes the hook's warning; B898's v0 mailer itself never fails and sends only the created event
#[async_trait]
pub trait KeyEventEmail: Send + Sync {
    async fn send_created(&self, response: CreatedKey) -> Result<(), KeyEmailError>;

    async fn send_rotated(
        &self,
        response: &str,
        existing_key_alias: Option<&str>,
    ) -> Result<(), KeyEmailError>;
}

#[async_trait]
impl KeyEventEmail for LegacyEmail<'_> {
    async fn send_created(&self, response: CreatedKey) -> Result<(), KeyEmailError> {
        self.send_key_created_email(response)
            .map_err(KeyEmailError::from)
    }

    async fn send_rotated(
        &self,
        _response: &str,
        _existing_key_alias: Option<&str>,
    ) -> Result<(), KeyEmailError> {
        self.send_key_rotated_email();
        Ok(())
    }
}

pub struct KeyEventEffects<'deps, Manager> {
    pub secrets: SecretSink<'deps, Manager>,
    pub email: &'deps dyn KeyEventEmail,
}
