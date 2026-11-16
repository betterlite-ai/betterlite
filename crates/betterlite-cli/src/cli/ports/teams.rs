use crate::commands::teams::{TeamsCommand, TeamsPorts, run_teams_command};
use crate::sso_polling::terminal::Terminal as SelectionTerminal;

use super::super::Failure;
use super::super::session::Session;
use super::wiring::message_sink;

pub(crate) fn run(session: &Session<'_>, command: TeamsCommand) -> Result<(), Failure> {
    let mut terminal = SelectionTerminal::default();
    let echo = message_sink(session.streams.stdout.clone());
    let mut stdout = session.streams.stdout.clone();
    let mut stderr = session.streams.stderr.clone();
    let ports = TeamsPorts {
        client: &session.client,
        terminal: &mut terminal,
        echo: &echo,
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    Ok(run_teams_command(command, ports)?)
}
