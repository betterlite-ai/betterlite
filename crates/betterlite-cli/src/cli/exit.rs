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

macro_rules! failure_from {
    (plain $($error:ty),* $(,)?) => {
        $(impl From<$error> for Failure {
            fn from(error: $error) -> Self {
                Self::Plain(error.to_string())
            }
        })*
    };
    (aborted $error:ty) => {
        impl From<$error> for Failure {
            fn from(error: $error) -> Self {
                Self::Aborted(error.to_string())
            }
        }
    };
    ($error:ty, usage = [$($usage:pat),* $(,)?], aborted = [$($aborted:pat),* $(,)?]) => {
        impl From<$error> for Failure {
            fn from(error: $error) -> Self {
                let message = error.to_string();
                if matches!(&error, $($usage)|*) {
                    Self::Usage(message)
                } else if matches!(&error, $($aborted)|*) {
                    Self::Aborted(message)
                } else {
                    Self::Plain(message)
                }
            }
        }
    };
    ($error:ty, usage = [$($usage:pat),* $(,)?]) => {
        impl From<$error> for Failure {
            fn from(error: $error) -> Self {
                let message = error.to_string();
                if matches!(&error, $($usage)|*) {
                    Self::Usage(message)
                } else {
                    Self::Plain(message)
                }
            }
        }
    };
}

failure_from!(aborted TeamsError);

failure_from!(
    ModelsError,
    usage = [
        ModelsError::Regex { .. },
        ModelsError::RegexMatch { .. },
        ModelsError::Pair { .. },
        ModelsError::Duplicate { .. },
        ModelsError::MissingSelector,
        ModelsError::BothSelectors,
    ]
);
failure_from!(
    CredentialsError,
    usage = [CredentialsError::InvalidJson { .. }],
    aborted = [CredentialsError::Aborted]
);
failure_from!(
    KeysError,
    usage = [KeysError::Usage(_)],
    aborted = [KeysError::Aborted]
);
failure_from!(
    HttpError,
    usage = [
        HttpError::HeaderFormat { .. },
        HttpError::InvalidJson { .. },
        HttpError::Method { .. }
    ],
    aborted = [HttpError::Aborted]
);
failure_from!(ConfigError, usage = [ConfigError::Usage(_)]);
failure_from!(
    ConfigureCommandError,
    usage = [ConfigureCommandError::Usage(_)]
);

failure_from!(plain
    ClientError,
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
    AgentDispatchError
);
