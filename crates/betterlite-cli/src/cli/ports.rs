use std::env::temp_dir;
use std::io;
use std::sync::Arc;

use betterlite_logging::private_json::files::PrivateFiles;

use crate::commands::chat::{
    ChatArgs, FileConversationStore, StdTerminal as ChatTerminal, run_chat_command,
};
use crate::commands::credentials::{CredentialsCommand, run_credentials_command};
use crate::commands::debug::{
    DebugCommand, DebugPorts, render::SystemClock as DebugClock, run as run_debug,
};
use crate::commands::encryption::{EncryptionCommand, EncryptionPorts, run_encryption_command};
use crate::commands::http::{HttpCommand, run_http_command};
use crate::commands::keys::{KeysCommand, KeysPorts, run_keys_command};
use crate::commands::model_groups::{ModelGroupsCommand, run_model_groups_command};
use crate::commands::models::{ModelsCommand, run_models_command};
use crate::commands::teams::{TeamsCommand, TeamsPorts, run_teams_command};
use crate::commands::users::{UsersCommand, run_users_command};
use crate::config::commands::{ConfigCommand, run_config_command};
use crate::session::auth_command::{AuthCommand, run_auth_command};
use crate::session::logout::{LogoutRequest, logout as run_logout};
use crate::session::print_token::PrintTokenRequest;
use crate::session::whoami::{WhoamiRequest, whoami as run_whoami};
use crate::settings_backup::down::down as restore_settings;
use crate::sso_polling::terminal::Terminal as SelectionTerminal;
use crate::statusline::{StatuslineArgs, StatuslinePorts, run as run_statusline};

use super::Failure;
use super::session::Session;
use super::version::print_version;
use super::wiring::{message_sink, token_file};

pub(crate) mod agent;
pub(crate) mod wiring;

pub(crate) fn logout(session: &mut Session<'_>) -> Result<(), Failure> {
    let files = token_file(session)?;
    let mut stdout = session.streams.stdout.clone();
    Ok(run_logout(&mut LogoutRequest {
        files: &files,
        vault: &mut *session.vault,
        http: Arc::clone(&session.client),
        token_path: &files.path,
        stdout: &mut stdout,
    })?)
}

pub(crate) fn whoami(session: &mut Session<'_>) -> Result<(), Failure> {
    let files = token_file(session)?;
    let mut stdout = session.streams.stdout.clone();
    Ok(run_whoami(&mut WhoamiRequest {
        files: &files,
        vault: &mut *session.vault,
        clock: Arc::clone(&session.clock),
        expiration_hours: session.expiration_hours,
        stdout: &mut stdout,
    })?)
}

pub(crate) fn auth(session: &mut Session<'_>, command: AuthCommand) -> Result<(), Failure> {
    let files = token_file(session)?;
    let mut stdout = session.streams.stdout.clone();
    let mut stderr = session.streams.stderr.clone();
    Ok(run_auth_command(
        command,
        &mut PrintTokenRequest {
            context: &session.context,
            files: &files,
            vault: &mut *session.vault,
            http: Arc::clone(&session.client),
            clock: Arc::clone(&session.clock),
            expiration_hours: session.expiration_hours,
            token_path: &files.path,
            stdout: &mut stdout,
            stderr: &mut stderr,
        },
    )?)
}

pub(crate) fn down(session: &mut Session<'_>) -> Result<(), Failure> {
    let files = PrivateFiles::default();
    Ok(restore_settings(
        session.environment,
        &files,
        &mut session.streams.stdout,
    )?)
}

pub(crate) fn models(session: &mut Session<'_>, command: ModelsCommand) -> Result<(), Failure> {
    Ok(run_models_command(
        command,
        &session.client,
        &mut session.streams.stdout,
        &mut session.streams.stderr,
    )?)
}

pub(crate) fn credentials(
    session: &mut Session<'_>,
    command: CredentialsCommand,
) -> Result<(), Failure> {
    Ok(run_credentials_command(
        command,
        &session.client,
        &mut session.streams.stdout,
        &mut session.streams.stderr,
    )?)
}

pub(crate) fn users(session: &mut Session<'_>, command: UsersCommand) -> Result<(), Failure> {
    Ok(run_users_command(
        command,
        &session.client,
        &mut session.streams.stdout,
    )?)
}

pub(crate) fn model_groups(
    session: &mut Session<'_>,
    command: ModelGroupsCommand,
) -> Result<(), Failure> {
    Ok(run_model_groups_command(
        command,
        &session.client,
        &mut session.streams.stdout,
    )?)
}

pub(crate) fn teams(session: &mut Session<'_>, command: TeamsCommand) -> Result<(), Failure> {
    let mut terminal = SelectionTerminal::default();
    let echo = message_sink(session.streams.stdout.clone());
    let mut stdout = session.streams.stdout.clone();
    let mut stderr = session.streams.stderr.clone();
    let mut ports = TeamsPorts {
        client: &session.client,
        terminal: &mut terminal,
        echo: &echo,
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    Ok(run_teams_command(command, &mut ports)?)
}

pub(crate) fn keys(session: &mut Session<'_>, command: KeysCommand) -> Result<(), Failure> {
    let mut ports = KeysPorts {
        context: &session.context,
        client: &session.client,
        redactor: &session.redactor,
        stdout: &mut session.streams.stdout,
        stderr: &mut session.streams.stderr,
    };
    Ok(run_keys_command(command, &mut ports)?)
}

pub(crate) fn encryption(
    session: &mut Session<'_>,
    command: EncryptionCommand,
) -> Result<(), Failure> {
    let mut ports = EncryptionPorts {
        context: &session.context,
        client: &session.client,
        redactor: &session.redactor,
        stdout: &mut session.streams.stdout,
        stderr: &mut session.streams.stderr,
    };
    Ok(run_encryption_command(command, &mut ports)?)
}

pub(crate) fn config(session: &mut Session<'_>, command: ConfigCommand) -> Result<(), Failure> {
    Ok(run_config_command(
        command,
        session.environment,
        &mut session.streams.stdout,
        &mut session.streams.stderr,
    )?)
}

pub(crate) fn chat(session: &mut Session<'_>, args: &ChatArgs) -> Result<(), Failure> {
    Ok(run_chat_command(
        args,
        &session.context,
        &session.client,
        &mut ChatTerminal,
        &mut FileConversationStore,
    )?)
}

pub(crate) fn http(session: &mut Session<'_>, command: HttpCommand) -> Result<(), Failure> {
    Ok(run_http_command(
        command,
        &session.context,
        &session.client,
        &mut session.streams.stdout,
        &mut session.streams.stderr,
    )?)
}

pub(crate) fn debug(session: &mut Session<'_>, command: DebugCommand) -> Result<(), Failure> {
    let clock = DebugClock;
    let mut stdout = session.streams.stdout.clone();
    let mut stderr = session.streams.stderr.clone();
    Ok(run_debug(
        command,
        DebugPorts {
            context: &session.context,
            client: &session.client,
            environment: session.environment,
            clock: &clock,
            stdout: &mut stdout,
            stderr: &mut stderr,
        },
    )?)
}

pub(crate) fn statusline(session: &mut Session<'_>, args: StatuslineArgs) -> Result<(), Failure> {
    let home = session
        .home
        .clone()
        .ok_or_else(|| Failure::Plain(String::from("cannot find your home directory")))?;
    let temp = temp_dir();
    let mut stdin = io::stdin();
    let mut ports = StatuslinePorts {
        stdin: &mut stdin,
        stdout: &mut session.streams.stdout,
        environment: &session.base_env,
        home_dir: &home,
        temp_dir: &temp,
    };
    Ok(run_statusline(args, &mut ports)?)
}

pub(crate) fn version(session: &mut Session<'_>) -> Result<(), Failure> {
    print_version(
        &session.context,
        &session.client,
        &mut session.streams.stdout,
    )
}
