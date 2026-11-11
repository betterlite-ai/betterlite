#![cfg(test)]

mod support;

use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup as _;
use rstest::rstest;
use secrecy::SecretString;

use self::support::{
    API_KEY, BASE_URL, CODEX_BINARY, CatalogUpgrade, DebugAnswer, FakeEnvironment, FileFailure,
    Harness, HttpAnswer, OpenCodeConfig, PreparerScript, RecordingMessages, ScriptedDebug,
    ScriptedFiles, ScriptedHttp, ScriptedInstaller, TerminalScript, command, files_in, key,
    limits_body, models_body, read_catalog, read_models_file, stock_catalog, stock_row, temp_dir,
    vars,
};
use crate::agent_launch::AgentRunError as LaunchError;
use crate::agent_launch::{EnvVars, Platform, WindowsCommand, agent_launch_args};
use crate::agent_run::codex::{
    CODEX_DEBUG_TIMEOUT, CODEX_MODEL_CATALOG_FILENAME, DebugFailure, DebugInvocation,
    DebugModels as _, DebugModelsOutcome, DebugOutput, StdDebugModels, StockModels,
    codex_debug_models, codex_model_catalog_path, replace_file, stock_codex_models,
};
use crate::agent_run::environment::BaseEnvironment;
use crate::agent_run::error::AgentRunError;
use crate::agent_run::messages::{Messages as _, NoticeBuffer, StreamMessages};
use crate::agent_run::prepare::{
    CodexPreparer, PiPreparer, PrepareRequest, Preparer as _, Preparers, StatuslineInstaller as _,
    StdStatuslineInstaller,
};
use crate::agent_run::resolver::{BinaryResolver as _, PathResolver};
use crate::agent_run::sync::{
    AgentModelSync, CodexSyncRequest, ModelSync as _, SKIP_VERIFY_FLAG, SyncContext, SyncRequest,
    agent_model_sync_env, codex_model_sync_args,
};
use crate::claude_settings::statusline_install::statusline_command;
use crate::opencode::{ModelSyncArgs, ModelSyncResult, ModelSyncSkipped};
use crate::pi::PiSyncError;

include!("tests/codex.rs");
include!("tests/sync.rs");
include!("tests/prepare.rs");
include!("tests/pipeline.rs");
include!("tests/resolver.rs");
include!("tests/messages.rs");
include!("tests/process.rs");
