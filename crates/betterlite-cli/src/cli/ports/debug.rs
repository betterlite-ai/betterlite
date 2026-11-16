use crate::commands::debug::{
    DebugCommand, DebugPorts, render::SystemClock as DebugClock, run as run_debug,
};

use super::super::Failure;
use super::super::session::Session;

pub(crate) fn run(session: &mut Session<'_>, command: DebugCommand) -> Result<(), Failure> {
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
