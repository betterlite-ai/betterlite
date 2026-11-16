use std::path::Path;
use std::sync::Arc;

use betterlite_logging::private_json::files::PrivateFiles;

use crate::agent_dispatch::{AgentCommandArgs, AgentPorts, run_agent_command};
use crate::agent_launch::Platform;
use crate::agent_run::codex::StdDebugModels;
use crate::agent_run::messages::StreamMessages;
use crate::agent_run::prepare::{CodexPreparer, PiPreparer, Preparers};
use crate::agent_run::sync::{AgentModelSync, SyncContext};
use crate::codex_catalog::CODEX_BASE_INSTRUCTIONS;
use crate::commands::Client;

use super::super::Failure;
use super::super::session::Session;
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
    let mut setup = Wiring::new(session)?;
    let client: &Client = &session.client;
    let pi = PiPreparer {
        get: client,
        files: &setup.private_files,
    };
    let codex = CodexPreparer {
        install: &setup.installer,
    };
    let preparers = Preparers {
        pi: &pi,
        codex: &codex,
    };
    let sync = model_sync(
        client,
        &setup.debug,
        &setup.private_files,
        session.home.as_deref(),
    );
    let mut messages = StreamMessages {
        notices: session.streams.stdout.clone(),
        warnings: session.streams.stderr.clone(),
    };
    let mut stdout = session.streams.stdout.clone();
    let mut ports = AgentPorts {
        context: &session.context,
        environment: session.environment,
        files: &setup.files,
        vault: &mut *session.vault,
        private_files: &setup.private_files,
        http: Arc::clone(&session.client),
        clock: Arc::clone(&session.clock),
        entropy: &mut setup.entropy,
        sleeper: &setup.sleeper,
        selection: &mut setup.selection,
        browser: &no_browser,
        interrupt: &setup.interrupt,
        token_path: &setup.token_path,
        binary_path: session.binary_path.as_deref(),
        base_env: &session.base_env,
        home: session.home.as_deref(),
        resolver: &setup.resolver,
        probe: client,
        sync: &sync,
        preparers: &preparers,
        launcher: &setup.launcher,
        platform: PLATFORM,
        terminal: Some(&setup.terminal),
        messages: &mut messages,
        stdout: &mut stdout,
    };
    let code = run_agent_command(binary, args, &mut ports)?;
    Ok(u8::try_from(code).unwrap_or(1))
}

fn model_sync<'sync>(
    client: &'sync Client,
    debug: &'sync StdDebugModels,
    files: &'sync PrivateFiles,
    home: Option<&'sync Path>,
) -> AgentModelSync<'sync> {
    AgentModelSync {
        context: SyncContext {
            get: client,
            debug,
            files,
            home,
            instructions: CODEX_BASE_INSTRUCTIONS,
        },
    }
}
