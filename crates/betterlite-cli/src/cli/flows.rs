use std::sync::Arc;

use crate::login::ports::{FlowPorts, LoginPorts};
use crate::login::{LoginArgs, run_login};
use crate::sso_polling::terminal::Terminal as SelectionTerminal;
#[cfg(unix)]
use crate::up::command::run_up;
#[cfg(unix)]
use crate::up::ports::{FlowLogin, UpPorts};
#[cfg(unix)]
use crate::up::signals::SignalStop;

use super::Failure;
use super::ports::wiring::{Wiring, message_sink, no_browser};
use super::session::{Session, SystemEntropy};

pub(crate) mod tools;

pub(crate) fn login(session: &mut Session<'_>, args: LoginArgs) -> Result<(), Failure> {
    let mut wiring = Wiring::new(session)?;
    let mut entropy = SystemEntropy;
    let mut selection = SelectionTerminal::default();
    let clock = Arc::clone(&session.clock);
    let mut stdout = session.streams.stdout.clone();
    let mut ports = LoginPorts {
        flow: FlowPorts {
            context: &session.context,
            environment: session.environment,
            files: &wiring.files,
            vault: &mut *session.vault,
            private_files: &wiring.private_files,
            http: Arc::clone(&session.client),
            clock: &*clock,
            wall_clock: &*clock,
            entropy: &mut entropy,
            sleeper: &wiring.sleeper,
            selection: &mut selection,
            browser: &no_browser,
            interrupt: &wiring.interrupt,
            token_path: &wiring.token_path,
            binary_path: session.binary_path.as_deref(),
        },
        stdout: &mut stdout,
    };
    Ok(run_login(args, &mut ports)?)
}

#[cfg(unix)]
pub(crate) fn up(session: &mut Session<'_>) -> Result<u8, Failure> {
    let mut wiring = Wiring::new(session)?;
    let mut entropy = SystemEntropy;
    let mut selection = SelectionTerminal::default();
    let mut login = FlowLogin;
    let signals = SignalStop::default();
    let warn = message_sink(session.streams.stderr.clone());
    let mut stdout = session.streams.stdout.clone();
    let mut stderr = session.streams.stderr.clone();
    let mut ports = UpPorts {
        context: &session.context,
        environment: session.environment,
        files: &wiring.files,
        vault: &mut *session.vault,
        private_files: &wiring.private_files,
        http: Arc::clone(&session.client),
        clock: Arc::clone(&session.clock),
        entropy: &mut entropy,
        sleeper: &wiring.sleeper,
        selection: &mut selection,
        browser: &no_browser,
        interrupt: &wiring.interrupt,
        token_path: &wiring.token_path,
        binary_path: session.binary_path.as_deref(),
        home: session.home.as_deref(),
        expiration_hours: session.expiration_hours,
        warn: &warn,
        terminal: Some(&wiring.terminal),
        login: &mut login,
        probe: &*session.client,
        signals: &signals,
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    run_up(&mut ports)?;
    Ok(0)
}
