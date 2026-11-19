use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_core::skills::sandbox::executor::{SandboxExecutor, SandboxOutcome, SandboxRequest};
use betterlite_core::skills::sandbox::session::{
    RunResult, SandboxSession, SdkFailure, SessionError, SessionOpener, SessionOptions,
};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::json;
use tokio::runtime::Builder;

const EXECUTION_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    code: String,
    #[serde(default)]
    files: BTreeMap<String, String>,
    #[serde(default)]
    requirements: Option<String>,
    #[serde(default)]
    image: Option<String>,
    #[serde(default)]
    install: Reply,
    #[serde(default)]
    execution: Reply,
    #[serde(default)]
    listing: Reply,
    #[serde(default)]
    runtime_files: BTreeMap<String, String>,
    #[serde(default)]
    refusing_copies: Vec<String>,
    #[serde(default)]
    missing_sdk: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    #[serde(default)]
    exit_code: i32,
    #[serde(default)]
    stdout: Option<String>,
    #[serde(default)]
    stderr: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Install,
    Execution,
    Listing,
}

fn kind_of(code: &str) -> Kind {
    if code.contains("os.walk('/sandbox')") {
        Kind::Listing
    } else if code.contains("'pip', 'install', '-r'") {
        Kind::Install
    } else {
        Kind::Execution
    }
}

#[derive(Debug, Default)]
struct Calls {
    session: Option<SessionOptions>,
    copies: Vec<(String, String)>,
    runs: Vec<String>,
    downloads: Vec<String>,
    closes: u32,
}

struct Script {
    input: Input,
    calls: Mutex<Calls>,
}

impl Script {
    fn calls(&self) -> MutexGuard<'_, Calls> {
        match self.calls.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn reply_for(&self, kind: Kind) -> Reply {
        let chosen = match kind {
            Kind::Install => &self.input.install,
            Kind::Execution => &self.input.execution,
            Kind::Listing => &self.input.listing,
        };
        chosen.clone()
    }

    fn effects(&self) -> serde_json::Value {
        let calls = self.calls();
        json!({
            "session": calls.session.as_ref().map(|options| json!({
                "lang": options.lang,
                "verbose": options.verbose,
                "image": &options.image,
            })),
            "copies": calls
                .copies
                .iter()
                .map(|(path, content)| json!({"sandbox_path": path, "content": content}))
                .collect::<Vec<_>>(),
            "runs": &calls.runs,
            "downloads": &calls.downloads,
            "closes": calls.closes,
        })
    }
}

fn session_error(operation: &'static str, message: String) -> SessionError {
    SessionError::Call {
        operation,
        source: SdkFailure::from(message),
    }
}

struct ScriptedSession {
    script: Arc<Script>,
    closed: AtomicBool,
}

#[async_trait]
impl SandboxSession for ScriptedSession {
    async fn copy_to_runtime(&self, local: &Path, sandbox_path: &str) -> Result<(), SessionError> {
        let bytes =
            fs::read(local).map_err(|error| session_error("copy_to_runtime", error.to_string()))?;
        let content = String::from_utf8(bytes)
            .map_err(|error| session_error("copy_to_runtime", error.to_string()))?;
        self.script
            .calls()
            .copies
            .push((sandbox_path.to_owned(), content));
        Ok(())
    }

    async fn copy_from_runtime(
        &self,
        source: &str,
        destination: &Path,
    ) -> Result<(), SessionError> {
        let input = &self.script.input;
        self.script.calls().downloads.push(source.to_owned());
        if input.refusing_copies.iter().any(|path| path == source) {
            fs::write(destination, b"partial")
                .map_err(|error| session_error("copy_from_runtime", error.to_string()))?;
            return Err(session_error(
                "copy_from_runtime",
                "the copy was refused".to_owned(),
            ));
        }
        let Some(content) = input.runtime_files.get(source) else {
            return Err(session_error(
                "copy_from_runtime",
                "no such file".to_owned(),
            ));
        };
        fs::write(destination, content)
            .map_err(|error| session_error("copy_from_runtime", error.to_string()))?;
        Ok(())
    }

    async fn run(&self, code: &str) -> Result<RunResult, SessionError> {
        let chosen = {
            let mut calls = self.script.calls();
            calls.runs.push(code.to_owned());
            self.script.reply_for(kind_of(code))
        };
        Ok(RunResult {
            exit_code: chosen.exit_code,
            stdout: chosen.stdout,
            stderr: chosen.stderr,
        })
    }

    async fn close(&self) -> Result<(), SessionError> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        self.script.calls().closes += 1;
        Ok(())
    }
}

impl Drop for ScriptedSession {
    fn drop(&mut self) {
        if !self.closed.swap(true, Ordering::SeqCst) {
            self.script.calls().closes += 1;
        }
    }
}

struct ScriptedOpener {
    script: Arc<Script>,
}

#[async_trait]
impl SessionOpener for ScriptedOpener {
    async fn open(
        &self,
        options: &SessionOptions,
    ) -> Result<Box<dyn SandboxSession>, SessionError> {
        if self.script.input.missing_sdk {
            return Err(SessionError::SdkMissing);
        }
        self.script.calls().session = Some(options.clone());
        Ok(Box::new(ScriptedSession {
            script: Arc::clone(&self.script),
            closed: AtomicBool::new(false),
        }))
    }
}

fn skill_files_of(input: &Input) -> IndexMap<String, Vec<u8>> {
    input
        .files
        .iter()
        .map(|(path, content)| (path.clone(), content.as_bytes().to_vec()))
        .collect()
}

fn outcome_json(outcome: &SandboxOutcome) -> serde_json::Value {
    json!({
        "success": outcome.success,
        "output": outcome.output,
        "error": outcome.error,
        "files": outcome
            .files
            .iter()
            .map(|file| json!({
                "name": &file.name,
                "path": &file.path,
                "content_base64": &file.content_base64,
                "mime_type": &file.mime_type,
            }))
            .collect::<Vec<_>>(),
    })
}

fn invalid_input(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let script = Arc::new(Script {
            input: input.clone(),
            calls: Mutex::new(Calls::default()),
        });
        let runtime = Builder::new_current_thread()
            .build()
            .map_err(|error| invalid_input(error.to_string()))?;
        let executor = SandboxExecutor {
            opener: ScriptedOpener {
                script: Arc::clone(&script),
            },
            image: input.image.clone(),
            timeout: EXECUTION_TIMEOUT,
        };
        let files = skill_files_of(input);
        let outcome = runtime.block_on(executor.execute(SandboxRequest {
            code: &input.code,
            skill_files: &files,
            requirements: input.requirements.as_deref(),
        }));
        Ok(json!({
            "result": outcome_json(&outcome),
            "effects": script.effects(),
        }))
    }
}
