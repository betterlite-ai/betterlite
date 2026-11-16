use clap::{Args as _, Command, Subcommand as _};

use crate::commands::credentials::CredentialsCommand;
use crate::commands::encryption::EncryptionCommand;
use crate::commands::keys::KeysCommand;
use crate::commands::model_groups::ModelGroupsCommand;
use crate::commands::models::ModelsCommand;
use crate::commands::teams::TeamsCommand;
use crate::commands::users::UsersCommand;
use crate::login::{LOGIN_ABOUT, LoginArgs};
use crate::session::auth_command::{AUTH_ABOUT, AuthCommand};
use crate::session::logout::LOGOUT_ABOUT;
use crate::session::whoami::WHOAMI_ABOUT;

pub(crate) fn commands() -> Vec<Command> {
    vec![
        LoginArgs::augment_args(Command::new("login").about(LOGIN_ABOUT)),
        Command::new("logout").about(LOGOUT_ABOUT),
        Command::new("whoami").about(WHOAMI_ABOUT),
        AuthCommand::augment_subcommands(
            Command::new("auth")
                .about(AUTH_ABOUT)
                .disable_help_subcommand(true),
        ),
        ModelsCommand::augment_subcommands(Command::new("models"))
            .about("Manage models on your betterlite proxy server"),
        CredentialsCommand::augment_subcommands(Command::new("credentials"))
            .about("Manage credentials for the betterlite proxy server"),
        UsersCommand::augment_subcommands(Command::new("users"))
            .about("Manage users on your betterlite proxy server"),
        TeamsCommand::augment_subcommands(Command::new("teams")),
        ModelGroupsCommand::augment_subcommands(Command::new("model-groups")),
        KeysCommand::augment_subcommands(Command::new("keys")),
        EncryptionCommand::augment_subcommands(
            Command::new("encryption").disable_help_subcommand(true),
        ),
    ]
}
