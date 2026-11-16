use std::io;

use super::Failure;
use crate::agent_dispatch::error::AgentDispatchError;
use crate::autoroute::commands::error::CommandsError;
use crate::commands::ClientError;
use crate::commands::chat::ChatError;
use crate::commands::configure::error::ConfigureCommandError;
use crate::commands::credentials::CredentialsError;
use crate::commands::debug::error::DebugError;
use crate::commands::encryption::EncryptionError;
use crate::commands::http::HttpError;
use crate::commands::keys::KeysError;
use crate::commands::model_groups::ModelGroupsError;
use crate::commands::models::ModelsError;
use crate::commands::teams::TeamsError;
use crate::commands::users::UsersError;
use crate::config::error::ConfigError;
use crate::login::error::LoginError;
use crate::session::error::SessionError;
use crate::settings_backup::error::BackupError;
use crate::statusline::error::StatuslineError;
use crate::up::error::UpError;

impl From<io::Error> for Failure {
    fn from(error: io::Error) -> Self {
        Self::Plain(format!("cannot write the output: {error}"))
    }
}

impl From<clap::Error> for Failure {
    fn from(error: clap::Error) -> Self {
        Self::Usage(error.render().to_string().trim_end().to_owned())
    }
}

impl From<ClientError> for Failure {
    fn from(error: ClientError) -> Self {
        Self::Plain(error.to_string())
    }
}

impl From<ModelsError> for Failure {
    fn from(error: ModelsError) -> Self {
        let message = error.to_string();
        match &error {
            ModelsError::Regex { .. }
            | ModelsError::RegexMatch { .. }
            | ModelsError::Pair { .. }
            | ModelsError::Duplicate { .. }
            | ModelsError::MissingSelector
            | ModelsError::BothSelectors => Self::Usage(message),
            _ => Self::Plain(message),
        }
    }
}

impl From<CredentialsError> for Failure {
    fn from(error: CredentialsError) -> Self {
        let message = error.to_string();
        match &error {
            CredentialsError::InvalidJson { .. } => Self::Usage(message),
            CredentialsError::Aborted => Self::Aborted(message),
            _ => Self::Plain(message),
        }
    }
}

impl From<KeysError> for Failure {
    fn from(error: KeysError) -> Self {
        let message = error.to_string();
        match &error {
            KeysError::Usage(_) => Self::Usage(message),
            KeysError::Aborted => Self::Aborted(message),
            _ => Self::Plain(message),
        }
    }
}

impl From<HttpError> for Failure {
    fn from(error: HttpError) -> Self {
        let message = error.to_string();
        match &error {
            HttpError::HeaderFormat { .. }
            | HttpError::InvalidJson { .. }
            | HttpError::Method { .. } => Self::Usage(message),
            HttpError::Aborted => Self::Aborted(message),
            _ => Self::Plain(message),
        }
    }
}

impl From<TeamsError> for Failure {
    fn from(error: TeamsError) -> Self {
        Self::Aborted(error.to_string())
    }
}

impl From<ConfigError> for Failure {
    fn from(error: ConfigError) -> Self {
        let message = error.to_string();
        match &error {
            ConfigError::Usage(_) => Self::Usage(message),
            _ => Self::Plain(message),
        }
    }
}

impl From<ConfigureCommandError> for Failure {
    fn from(error: ConfigureCommandError) -> Self {
        let message = error.to_string();
        match &error {
            ConfigureCommandError::Usage(_) => Self::Usage(message),
            _ => Self::Plain(message),
        }
    }
}

macro_rules! plain_failures {
    ($($error:ty),* $(,)?) => {
        $(impl From<$error> for Failure {
            fn from(error: $error) -> Self {
                Self::Plain(error.to_string())
            }
        })*
    };
}

plain_failures!(
    UsersError,
    ModelGroupsError,
    EncryptionError,
    DebugError,
    ChatError,
    LoginError,
    SessionError,
    UpError,
    BackupError,
    CommandsError,
    StatuslineError,
    AgentDispatchError,
);
