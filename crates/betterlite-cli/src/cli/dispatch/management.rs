use clap::ArgMatches;
use clap::FromArgMatches as _;

use crate::commands::credentials::CredentialsCommand;
use crate::commands::encryption::EncryptionCommand;
use crate::commands::keys::KeysCommand;
use crate::commands::model_groups::ModelGroupsCommand;
use crate::commands::models::ModelsCommand;
use crate::commands::teams::TeamsCommand;
use crate::commands::users::UsersCommand;
use crate::config::commands::ConfigCommand;

use super::super::Failure;
use super::super::ports;
use super::super::session::Session;

pub(crate) fn run(session: &mut Session<'_>, name: &str, sub: &ArgMatches) -> Result<u8, Failure> {
    let run = match name {
        "models" => ports::models::run(session, ModelsCommand::from_arg_matches(sub)?),
        "credentials" => {
            ports::credentials::run(session, CredentialsCommand::from_arg_matches(sub)?)
        }
        "users" => ports::users::run(session, UsersCommand::from_arg_matches(sub)?),
        "teams" => ports::teams::run(session, TeamsCommand::from_arg_matches(sub)?),
        "model-groups" => {
            ports::model_groups::run(session, ModelGroupsCommand::from_arg_matches(sub)?)
        }
        "keys" => ports::keys::run(session, KeysCommand::from_arg_matches(sub)?),
        "encryption" => ports::encryption::run(session, EncryptionCommand::from_arg_matches(sub)?),
        "config" => ports::config::run(session, ConfigCommand::from_arg_matches(sub)?),
        _unknown => return Ok(0),
    };
    run.map(|()| 0)
}
