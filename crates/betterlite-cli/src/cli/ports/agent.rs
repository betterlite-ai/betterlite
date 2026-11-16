use std::path::Path;
use std::sync::Arc;

use betterlite_logging::private_json::files::LocalFiles;

use crate::agent_dispatch::{AgentCommandArgs, AgentPorts, run_agent_command};
use crate::agent_launch::Platform;
use crate::agent_run::codex::StdDebugModels;
use crate::agent_run::messages::StreamMessages;
use crate::agent_run::prepare::{CodexPreparer, PiPreparer, Preparers};
use crate::agent_run::sync::{AgentModelSync, SyncContext};
use crate::codex_catalog::CODEX_BASE_INSTRUCTIONS;
use crate::commands::Client;
use crate::sso_polling::terminal::Terminal as SelectionTerminal;

use super::super::Failure;
use super::super::session::{Session, SystemEntropy};
use super::super::streams::{Channel, Streams};
use super::wiring::{Wiring, no_browser};

#[cfg(unix)]
const PLATFORM: Platform = Platform::Unix;
#[cfg(not(unix))]
const PLATFORM: Platform = Platform::Windows;

pub(crate) fn run(
    session: &mut Session<'_>,
    binary: &str,
    args: AgentCommandArgs,
) -> Result<u8, Failure> {
    let setup = Wiring::new(session)?;
    let mut entropy = SystemEntropy;
    let mut selection = SelectionTerminal::default();
    let home = session.home.as_deref();
    let mut prep = Preparation::new(&session.client, &setup, home, &session.streams);
    let preparers = Preparers {
        pi: &prep.pi,
        codex: &prep.codex,
    };
    let mut ports = AgentPorts {
        context: &session.context,
        environment: session.environment,
        files: &setup.files,
        vault: &mut *session.vault,
        private_files: &setup.private_files,
        http: Arc::clone(&session.client),
        clock: Arc::clone(&session.clock),
        entropy: &mut entropy,
        sleeper: &setup.sleeper,
        selection: &mut selection,
        browser: &no_browser,
        interrupt: &setup.interrupt,
        token_path: &setup.token_path,
        binary_path: session.binary_path.as_deref(),
        base_env: &session.base_env,
        home: session.home.as_deref(),
        resolver: &setup.resolver,
        probe: &*session.client,
        sync: &prep.sync,
        preparers: &preparers,
        launcher: &setup.launcher,
        platform: PLATFORM,
        terminal: Some(&setup.terminal),
        messages: &mut prep.messages,
        stdout: &mut prep.stdout,
    };
    let code = run_agent_command(binary, args, &mut ports)?;
    Ok(u8::try_from(code).unwrap_or(1))
}

struct Preparation<'prepared> {
    pi: PiPreparer<'prepared>,
    codex: CodexPreparer<'prepared>,
    sync: AgentModelSync<'prepared>,
    messages: StreamMessages<Channel, Channel>,
    stdout: Channel,
}

impl<'prepared> Preparation<'prepared> {
    fn new(
        client: &'prepared Client,
        setup: &'prepared Wiring,
        home: Option<&'prepared Path>,
        streams: &Streams,
    ) -> Self {
        Self {
            pi: PiPreparer {
                get: client,
                files: &LocalFiles,
            },
            codex: CodexPreparer {
                install: &setup.installer,
            },
            sync: model_sync(client, &setup.debug, home),
            messages: StreamMessages {
                notices: streams.stdout.clone(),
                warnings: streams.stderr.clone(),
            },
            stdout: streams.stdout.clone(),
        }
    }
}

fn model_sync<'sync>(
    client: &'sync Client,
    debug: &'sync StdDebugModels,
    home: Option<&'sync Path>,
) -> AgentModelSync<'sync> {
    AgentModelSync {
        context: SyncContext {
            get: client,
            debug,
            files: &LocalFiles,
            home,
            instructions: CODEX_BASE_INSTRUCTIONS,
        },
    }
}
