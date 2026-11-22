use std::fmt::{Debug, Formatter, Result as FmtResult};

use betterlite_secret_managers::secret_contracts::BaseSecretManager;

use crate::audit_events::record::AuditRecordError;
use crate::key_event_sinks::error::SinkError;
use crate::key_event_sinks::team_params::TeamSecretSettings;
use crate::key_fields::error::DecodeError;

use super::effects::KeyEmailError;

// Preparing an event can fail and the hook then propagates the failure; a sink failure that LiteLLM swallows is a warning inside the hook and never reaches this type. The secret manager's own error type is a parameter of B898's sink, so the effect failure carries the sink's typed error.
#[derive(thiserror::Error)]
pub enum KeyEventError<Manager>
where
    Manager: BaseSecretManager,
    Manager::Params: TryFrom<TeamSecretSettings>,
{
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
        source: SinkError<Manager>,
    },
}

// the manager is the error's type parameter, not one of its fields, so a derived Debug would demand Manager: Debug and the scripted managers are not
impl<Manager> Debug for KeyEventError<Manager>
where
    Manager: BaseSecretManager,
    Manager::Params: TryFrom<TeamSecretSettings>,
{
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::Encode(source) => formatter.debug_tuple("Encode").field(source).finish(),
            Self::Row(source) => formatter.debug_tuple("Row").field(source).finish(),
            Self::EmailEvent(source) => formatter.debug_tuple("EmailEvent").field(source).finish(),
            Self::Email(source) => formatter.debug_tuple("Email").field(source).finish(),
            Self::Audit(source) => formatter.debug_tuple("Audit").field(source).finish(),
            Self::MissingUpdateKey => formatter.write_str("MissingUpdateKey"),
            Self::MissingResponseKey => formatter.write_str("MissingResponseKey"),
            Self::Sink { source } => formatter
                .debug_struct("Sink")
                .field("source", &format_args!("{source}"))
                .finish(),
        }
    }
}
