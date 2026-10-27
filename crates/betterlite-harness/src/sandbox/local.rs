mod tests;

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use secrecy::ExposeSecret;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex as AsyncMutex;

use crate::errors::{CappedOperation, HarnessError};
use crate::sandbox::{CompletedRun, Process, Sandbox, SandboxCommand};
use crate::snapshots::snapshot_local;

const PROCESS_KILL_GRACE_SECONDS: u64 = 5;
const TEMPDIR_PREFIX: &str = "litellm-harness-";
const SECRET_PREFIXES: [&str; 11] = [
    "ANTHROPIC_",
    "OPENAI_",
    "LITELLM_",
    "AZURE_",
    "AWS_",
    "GEMINI_",
    "CODEX_",
    "CURSOR_",
    "VERTEX",
    "CLAUDE_CODE_",
    "CLAUDE_PID",
];
const SECRET_NAMES: [&str; 2] = ["GOOGLE_API_KEY", "GOOGLE_APPLICATION_CREDENTIALS"];
const SECRET_SUBSTRINGS: [&str; 3] = ["API_KEY", "TOKEN", "SECRET"];

#[derive(Clone)]
struct ProcessSlot(Arc<AsyncMutex<Child>>);

pub struct SubprocessHandle {
    child: ProcessSlot,
    stdin: Option<ChildStdin>,
    stdout: ChildStdout,
    stderr: ChildStderr,
}

impl Process for SubprocessHandle {
    type Stdin = ChildStdin;
    type Stdout = ChildStdout;
    type Stderr = ChildStderr;

    fn stdin(&mut self) -> Option<&mut ChildStdin> {
        self.stdin.as_mut()
    }

    fn stdout(&mut self) -> &mut ChildStdout {
        &mut self.stdout
    }

    fn stderr(&mut self) -> &mut ChildStderr {
        &mut self.stderr
    }

    fn wait(&mut self) -> impl Future<Output = Result<i32, HarnessError>> + Send {
        let child = self.child.0.clone();
        async move {
            let mut child = child.lock().await;
            let status = child.wait().await.map_err(sandbox_error)?;
            Ok(status.code().unwrap_or(-1))
        }
    }

    fn kill(&mut self) -> impl Future<Output = Result<(), HarnessError>> + Send {
        kill_child(self.child.0.clone())
    }
}

struct State {
    closed: bool,
    processes: Vec<ProcessSlot>,
    tempdirs: Vec<PathBuf>,
}

pub struct LocalSandbox {
    workdir: String,
    state: Mutex<State>,
}

impl LocalSandbox {
    pub fn new(path: impl AsRef<Path>) -> Result<Self, HarnessError> {
        let resolved = real_path(path.as_ref()).map_err(sandbox_error)?;
        if !resolved.is_dir() {
            return Err(message_error(format!(
                "sandbox path does not exist or is not a directory: {}",
                resolved.display()
            )));
        }
        Ok(Self {
            workdir: resolved.to_string_lossy().into_owned(),
            state: Mutex::new(State {
                closed: false,
                processes: Vec::new(),
                tempdirs: Vec::new(),
            }),
        })
    }

    pub fn resolve_path(&self, path: &str) -> Result<String, HarnessError> {
        let path = Path::new(path);
        let joined = if path.is_absolute() {
            path.to_path_buf()
        } else {
            Path::new(&self.workdir).join(path)
        };
        let resolved = real_path(&joined).map_err(sandbox_error)?;
        let allowed = self.allowed_roots()?;
        if !allowed.iter().any(|root| within(&resolved, root)) {
            return Err(message_error(format!("path escapes the sandbox: {path:?}")));
        }
        Ok(resolved.to_string_lossy().into_owned())
    }

    pub fn child_env(
        &self,
        extra: &BTreeMap<String, secrecy::SecretString>,
    ) -> BTreeMap<String, String> {
        let mut result: BTreeMap<String, String> = env::vars()
            .filter(|(name, _)| !is_secret_env_name(name))
            .collect();
        for (name, value) in extra {
            result.insert(name.clone(), value.expose_secret().to_owned());
        }
        result
    }

    fn allowed_roots(&self) -> Result<Vec<PathBuf>, HarnessError> {
        let state = self
            .state
            .lock()
            .map_err(|_| message_error("sandbox state poisoned"))?;
        Ok(std::iter::once(PathBuf::from(&self.workdir))
            .chain(state.tempdirs.iter().cloned())
            .collect())
    }

    fn check_open(&self) -> Result<(), HarnessError> {
        let state = self
            .state
            .lock()
            .map_err(|_| message_error("sandbox state poisoned"))?;
        if state.closed {
            Err(message_error("sandbox is closed"))
        } else {
            Ok(())
        }
    }

    async fn collect(
        &self,
        mut process: SubprocessHandle,
        command: &SandboxCommand,
        timeout: Duration,
        max: usize,
    ) -> Result<CompletedRun, HarnessError> {
        if let Some(stdin) = process.stdin.take() {
            drop(stdin);
        }
        let output = async {
            let (stdout, stderr) = tokio::join!(
                read_capped(&mut process.stdout, max),
                read_capped(&mut process.stderr, max)
            );
            let stdout = stdout?;
            let stderr = stderr?;
            let code = process.wait().await?;
            Ok::<_, HarnessError>((stdout, stderr, code))
        };
        let (stdout, stderr, exit_code) = match tokio::time::timeout(timeout, output).await {
            Ok(result) => result?,
            Err(_) => {
                process.kill().await?;
                let name = command.argv.first().map_or("", String::as_str);
                return Err(message_error(format!(
                    "command timed out after {:?}: {name}",
                    timeout
                )));
            }
        };
        Ok(CompletedRun {
            stdout: decode_utf8(&stdout),
            stderr: decode_utf8(&stderr),
            exit_code,
        })
    }
}

impl Sandbox for LocalSandbox {
    type Process = SubprocessHandle;

    fn workdir(&self) -> &str {
        &self.workdir
    }

    fn exec(
        &self,
        command: &SandboxCommand,
    ) -> impl Future<Output = Result<Self::Process, HarnessError>> + Send {
        let result = self.start(command);
        async move { result.await }
    }

    fn run(
        &self,
        command: &SandboxCommand,
        timeout: Duration,
        max_output_bytes: usize,
    ) -> impl Future<Output = Result<CompletedRun, HarnessError>> + Send {
        let command = command.clone();
        async move {
            let process = self.exec(&command).await?;
            self.collect(process, &command, timeout, max_output_bytes)
                .await
        }
    }

    fn read(
        &self,
        path: &str,
        max_bytes: usize,
    ) -> impl Future<Output = Result<Vec<u8>, HarnessError>> + Send {
        let result = self.check_open().and_then(|_| self.resolve_path(path));
        async move {
            let resolved = result?;
            let data = tokio::task::spawn_blocking(move || fs::read(resolved))
                .await
                .map_err(join_error)?
                .map_err(sandbox_error)?;
            if data.len() > max_bytes {
                return Err(HarnessError::ContentLimitExceeded {
                    operation: CappedOperation::Read,
                    limit: max_bytes,
                });
            }
            Ok(data)
        }
    }

    fn write(
        &self,
        path: &str,
        data: &[u8],
    ) -> impl Future<Output = Result<(), HarnessError>> + Send {
        let result = self.check_open().and_then(|_| self.resolve_path(path));
        let data = data.to_vec();
        async move {
            let resolved = result?;
            tokio::task::spawn_blocking(move || write_file(&resolved, &data))
                .await
                .map_err(join_error)?
                .map_err(sandbox_error)
        }
    }

    fn host_url(&self, port: u16) -> String {
        format!("http://127.0.0.1:{port}")
    }

    fn which(
        &self,
        binary: &str,
    ) -> impl Future<Output = Result<Option<String>, HarnessError>> + Send {
        let path = self.child_env(&BTreeMap::new()).get("PATH").cloned();
        let binary = binary.to_owned();
        async move { Ok(path.and_then(|path| find_binary(&binary, &path))) }
    }

    fn snapshot(
        &self,
    ) -> impl Future<Output = Result<BTreeMap<String, String>, HarnessError>> + Send {
        let root = PathBuf::from(self.workdir());
        async move {
            snapshot_local(root)
                .await
                .map_err(|_| message_error("workspace snapshot failed"))
        }
    }

    fn tempdir(&self) -> impl Future<Output = Result<String, HarnessError>> + Send {
        let result = self.check_open();
        async move {
            result?;
            let path = tokio::task::spawn_blocking(|| tempfile_path())
                .await
                .map_err(join_error)?
                .map_err(sandbox_error)?;
            let mut state = self
                .state
                .lock()
                .map_err(|_| message_error("sandbox state poisoned"))?;
            state.tempdirs.push(path.clone());
            Ok(path.to_string_lossy().into_owned())
        }
    }

    fn close(&self) -> impl Future<Output = Result<(), HarnessError>> + Send {
        async move {
            let Some((processes, tempdirs)) = self.take_close_state()? else {
                return Ok(());
            };
            for process in processes {
                drop(kill_child(process.0).await);
            }
            tokio::task::spawn_blocking(|| cleanup_tempdirs(tempdirs))
                .await
                .map_err(join_error)?;
            Ok(())
        }
    }
}

impl LocalSandbox {
    fn take_close_state(&self) -> Result<Option<(Vec<ProcessSlot>, Vec<PathBuf>)>, HarnessError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| message_error("sandbox state poisoned"))?;
        if state.closed {
            return Ok(None);
        }
        state.closed = true;
        Ok(Some((
            state.processes.drain(..).collect(),
            state.tempdirs.drain(..).collect(),
        )))
    }

    async fn start(&self, command: &SandboxCommand) -> Result<SubprocessHandle, HarnessError> {
        self.check_open()?;
        let name = command
            .argv
            .first()
            .ok_or_else(|| message_error("exec() needs a non-empty command"))?;
        let cwd = match &command.cwd {
            Some(path) => self.resolve_path(path)?,
            None => self.workdir.clone(),
        };
        if !Path::new(&cwd).is_dir() {
            return Err(message_error(format!("cwd is not a directory: {cwd}")));
        }
        let mut process = Command::new(name);
        process
            .args(command.argv.iter().skip(1))
            .current_dir(cwd)
            .envs(self.child_env(&command.env))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        #[cfg(unix)]
        process.process_group(0);
        let child = process
            .spawn()
            .map_err(|source| message_error(format!("could not start {name}: {source}")))?;
        let slot = ProcessSlot(Arc::new(AsyncMutex::new(child)));
        let (stdin, stdout, stderr) = {
            let mut child = slot.0.lock().await;
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        };
        let (Some(stdout), Some(stderr)) = (stdout, stderr) else {
            return Err(message_error(
                "subprocess was started without stdout/stderr pipes",
            ));
        };
        let handle = SubprocessHandle {
            child: slot.clone(),
            stdin,
            stdout,
            stderr,
        };
        let mut state = self
            .state
            .lock()
            .map_err(|_| message_error("sandbox state poisoned"))?;
        state.processes.push(slot);
        Ok(handle)
    }
}

pub fn local(path: impl AsRef<Path>) -> Result<LocalSandbox, HarnessError> {
    LocalSandbox::new(path)
}

pub fn is_secret_env_name(name: &str) -> bool {
    let upper = name.to_uppercase();
    upper == "CLAUDECODE"
        || SECRET_NAMES.contains(&upper.as_str())
        || SECRET_PREFIXES
            .iter()
            .any(|prefix| upper.starts_with(prefix))
        || SECRET_SUBSTRINGS.iter().any(|part| upper.contains(part))
}

pub fn filtered_environ(
    extra: &BTreeMap<String, secrecy::SecretString>,
) -> BTreeMap<String, String> {
    let mut kept: BTreeMap<String, String> = env::vars()
        .filter(|(name, _)| !is_secret_env_name(name))
        .collect();
    for (name, value) in extra {
        kept.insert(name.clone(), value.expose_secret().to_owned());
    }
    kept
}

async fn kill_child(child: Arc<AsyncMutex<Child>>) -> Result<(), HarnessError> {
    let process_id = {
        let mut child = child.lock().await;
        if child.try_wait().map_err(sandbox_error)?.is_some() {
            return Ok(());
        }
        child
            .id()
            .ok_or_else(|| message_error("running process has no pid"))?
    };
    send_signal(process_id, "TERM").await?;
    let wait = async {
        let mut child = child.lock().await;
        child.wait().await.map_err(sandbox_error)
    };
    if tokio::time::timeout(Duration::from_secs(PROCESS_KILL_GRACE_SECONDS), wait)
        .await
        .is_err()
    {
        send_signal(process_id, "KILL").await?;
        let mut child = child.lock().await;
        child.wait().await.map_err(sandbox_error)?;
    }
    Ok(())
}

async fn send_signal(process_id: u32, signal: &str) -> Result<(), HarnessError> {
    let signal = signal.to_owned();
    tokio::task::spawn_blocking(move || {
        let group = format!("-{process_id}");
        let status = StdCommand::new("kill")
            .args([signal.as_str(), "--", &group])
            .status()
            .map_err(sandbox_error)?;
        if status.success() {
            Ok(())
        } else {
            Err(message_error("could not signal sandbox process"))
        }
    })
    .await
    .map_err(join_error)?
}

async fn read_capped(
    reader: &mut (impl AsyncRead + Unpin),
    max: usize,
) -> Result<Vec<u8>, HarnessError> {
    let mut data = Vec::new();
    reader
        .take(u64::try_from(max).unwrap_or(u64::MAX).saturating_add(1))
        .read_to_end(&mut data)
        .await
        .map_err(sandbox_error)?;
    if data.len() > max {
        return Err(HarnessError::ContentLimitExceeded {
            operation: CappedOperation::RunOutput,
            limit: max,
        });
    }
    Ok(data)
}

fn real_path(path: &Path) -> io::Result<PathBuf> {
    if let Ok(path) = fs::canonicalize(path) {
        return Ok(path);
    }
    let mut missing = Vec::new();
    let mut current = path;
    let existing = loop {
        if let Ok(existing) = fs::canonicalize(current) {
            break existing;
        }
        let name = current
            .file_name()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        missing.push(name.to_owned());
        current = current
            .parent()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
    };
    Ok(missing
        .into_iter()
        .rev()
        .fold(existing, |path, part| path.join(part)))
}

fn within(path: &Path, root: &Path) -> bool {
    path == root || path.starts_with(root)
}
fn write_file(path: &str, data: &[u8]) -> io::Result<()> {
    if let Some(parent) = Path::new(path).parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, data)
}
fn tempfile_path() -> io::Result<PathBuf> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_nanos();
    let path = env::temp_dir().join(format!("{TEMPDIR_PREFIX}{stamp}"));
    fs::create_dir(&path)?;
    Ok(path)
}

fn cleanup_tempdirs(tempdirs: Vec<PathBuf>) {
    for path in tempdirs {
        drop(fs::remove_dir_all(path));
    }
}
fn find_binary(binary: &str, path: &str) -> Option<String> {
    env::split_paths(path)
        .map(|dir| dir.join(binary))
        .find(|candidate| candidate.is_file())
        .map(|candidate| candidate.to_string_lossy().into_owned())
}

fn decode_utf8(bytes: &[u8]) -> String {
    let mut decoded = String::new();
    let mut offset = 0;
    while offset < bytes.len() {
        match std::str::from_utf8(&bytes[offset..]) {
            Ok(text) => {
                decoded.push_str(text);
                break;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                decoded.push_str(std::str::from_utf8(&bytes[offset..offset + valid]).unwrap_or(""));
                decoded.push('\u{FFFD}');
                offset += valid + error.error_len().unwrap_or(1);
            }
        }
    }
    decoded
}
fn sandbox_error(source: io::Error) -> HarnessError {
    HarnessError::Sandbox { source }
}
fn join_error(source: tokio::task::JoinError) -> HarnessError {
    message_error(format!("sandbox worker failed: {source}"))
}
fn message_error(message: impl Into<String>) -> HarnessError {
    HarnessError::Sandbox {
        source: io::Error::new(io::ErrorKind::Other, message.into()),
    }
}
