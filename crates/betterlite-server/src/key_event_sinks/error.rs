use betterlite_secret_managers::secret_contracts::{BaseSecretManager, RotationError};

use super::team_params::TeamSecretSettings;

#[derive(Debug, thiserror::Error)]
pub enum SecretSinkError<Manager, Params> {
    #[error("the team's secret_manager_settings do not fit the secret manager's parameters")]
    Params {
        #[source]
        source: Params,
    },
    #[error("the secret manager call failed")]
    Manager {
        #[source]
        source: Manager,
    },
    #[error("the secret manager rotation failed")]
    Rotation {
        #[source]
        source: RotationError<Manager>,
    },
}

pub type SinkError<Manager> = SecretSinkError<
    <Manager as BaseSecretManager>::Error,
    <<Manager as BaseSecretManager>::Params as TryFrom<TeamSecretSettings>>::Error,
>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EmailError {
    // WebhookEvent declares spend a required float, so an explicit null fails its validation
    #[error("the key created email needs a spend, and the response holds null")]
    NullSpend,
}
