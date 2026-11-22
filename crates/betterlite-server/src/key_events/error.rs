use std::error::Error;

use betterlite_secret_managers::secret_contracts::BaseSecretManager;

use crate::audit_events::record::AuditRecordError;
use crate::key_event_sinks::error::SinkError;
use crate::key_event_sinks::team_params::TeamSecretSettings;
use crate::key_fields::error::DecodeError;

use super::effects::KeyEmailError;

// Preparing an event can fail and the hook then propagates the failure; a sink failure that LiteLLM swallows is a warning inside the hook and never reaches this type. The secret manager's own error type is a parameter of B898's sink, so the effect failure carries it as a boxed source.
#[derive(Debug, thiserror::Error)]
pub enum KeyEventError {
    #[error("the key event cannot be encoded")]
    Encode(#[source] DecodeError),
    #[error("the key row cannot be encoded")]
    Row(#[source] serde_json::Error),
    #[error("the key response cannot be read as a created-email event")]
    EmailEvent(#[source] serde_json::Error),
    #[error("the key email effect failed")]
    Email(#[source] KeyEmailError),
    #[error("the key audit record cannot be built")]
    Audit(#[source] AuditRecordError),
    #[error("the updated key request carries no key")]
    MissingUpdateKey,
    #[error("the rotated key response carries no key")]
    MissingResponseKey,
    #[error("the secret manager effect failed: {source}")]
    Sink {
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
}

impl<Manager> From<SinkError<Manager>> for KeyEventError
where
    Manager: BaseSecretManager,
    Manager::Params: TryFrom<TeamSecretSettings>,
    <Manager::Params as TryFrom<TeamSecretSettings>>::Error: Error + Send + Sync + 'static,
{
    fn from(source: SinkError<Manager>) -> Self {
        Self::Sink {
            source: Box::new(source),
        }
    }
}
