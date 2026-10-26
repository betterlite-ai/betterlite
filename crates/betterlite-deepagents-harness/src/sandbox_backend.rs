pub mod bridge;
pub mod errors;
mod execute;
mod files;
pub mod include_glob;
pub mod protocol;
pub mod read_window;
pub mod replace;
mod search;
mod tests;
mod transfer;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_harness::errors::HarnessError;
use betterlite_harness::sandbox::{CompletedRun, Sandbox, SandboxCommand};
use tokio::runtime::Handle;
use tokio::sync::OnceCell;
use uuid::Uuid;

use crate::sandbox_backend::errors::BackendError;

pub const EXECUTE_TIMEOUT_SECONDS: u64 = 120;
pub const FS_TIMEOUT_SECONDS: u64 = 60;
pub const MAX_OUTPUT_BYTES: usize = 100_000;
// the Rust Sandbox needs caps LiteLLM leaves open: the 50 MiB snapshot file limit for reads
const READ_CAP_BYTES: usize = 52_428_800;
// and 16 MiB of command output, well above what execute keeps
const RUN_CAP_BYTES: usize = 0x0100_0000;
const EXIT_NOT_FOUND: i32 = 3;
const EXIT_NOT_DIR: i32 = 4;
const EXIT_TIMEOUT: i32 = 124;
// GNU and BSD xargs exit 123 when a grep batch found nothing
const EXIT_XARGS_BATCH: i32 = 123;
const READ_ONLY_ERROR: &str = "Error: this session is read-only; files cannot be changed";
const NO_EXECUTE_ERROR: &str = "Error: shell execution is disabled for this session";
// $1 is the directory; prints "d/<name>" or "f/<name>" per entry, and "/" never appears in a name
const LS_SCRIPT: &str = concat!(
    r#"[ -e "$1" ] || exit 3; [ -d "$1" ] || exit 4; cd "$1" || exit 5; "#,
    r#"for f in * .[!.]* ..?*; do if [ -e "$f" ] || [ -L "$f" ]; then "#,
    r#"if [ -d "$f" ]; then printf "d/%s\n" "$f"; else printf "f/%s\n" "$f"; fi; fi; done"#,
);
const DELETE_SCRIPT: &str = r#"[ -e "$1" ] || [ -L "$1" ] || exit 3; rm -rf -- "$1""#;
// resolves the nearest existing ancestor, so a new file under a symlinked directory is still caught
const REALPATH_SCRIPT: &str = concat!(
    r#"p="$1"; while [ ! -e "$p" ] && [ ! -L "$p" ]; do q=$(dirname -- "$p"); "#,
    r#"[ "$q" = "$p" ] && exit 3; p="$q"; done; realpath -- "$p""#,
);
// $1 is the fixed-string pattern and the rest is the find command listing the files to search
const GREP_SCRIPT: &str =
    r#"pattern="$1"; shift; "$@" | tr "\n" "\0" | xargs -0 grep -nHFI -e "$pattern" --"#;
const GREP_LINE: &str = r"^(.+?):(\d+):(.*)$";
// LiteLLM's HARNESS_SNAPSHOT_SKIP_DIRS in sorted order
const SKIP_DIRS: [&str; 8] = [
    ".git",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    ".venv",
    "__pycache__",
    "node_modules",
    "venv",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Permissions {
    pub writable: bool,
    pub allow_execute: bool,
}

#[derive(Debug)]
pub struct SandboxBackend<S> {
    id: String,
    sandbox: Arc<S>,
    runtime: Handle,
    root: String,
    real_root: OnceCell<String>,
    permissions: Permissions,
}

impl<S: Sandbox> SandboxBackend<S> {
    #[must_use]
    pub fn new(sandbox: Arc<S>, runtime: Handle, permissions: Permissions, session: Uuid) -> Self {
        let mut short = session.simple().to_string();
        short.truncate(8);
        Self {
            id: format!("litellm-harness-{short}"),
            root: normalize(sandbox.workdir()),
            sandbox,
            runtime,
            real_root: OnceCell::new(),
            permissions,
        }
    }

    #[must_use]
    pub fn to_real(&self, path: &str) -> String {
        let normalized = normalize(path);
        if normalized == self.root
            || normalized
                .strip_prefix(self.root.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
        {
            normalized
        } else if normalized == "/" {
            self.root.clone()
        } else {
            format!("{}{normalized}", self.root)
        }
    }

    #[must_use]
    pub fn to_virtual(&self, real: &str) -> String {
        match real.strip_prefix(self.root.as_str()) {
            Some("") => "/".to_owned(),
            Some(rest) if rest.starts_with('/') => rest.to_owned(),
            Some(_) | None => real.to_owned(),
        }
    }

    pub async fn to_confined(&self, path: &str) -> Result<String, BackendError> {
        let real = self.to_real(path);
        let done = self
            .run(
                &["sh", "-c", REALPATH_SCRIPT, "sh", &real],
                FS_TIMEOUT_SECONDS,
            )
            .await?;
        let resolved = done.stdout.trim();
        if done.exit_code != 0_i32 || resolved.is_empty() {
            return Err(BackendError::NotFound {
                path: path.to_owned(),
            });
        }
        // resolved once and kept, so a root moved mid-session is not seen
        let root = self
            .real_root
            .get_or_try_init(|| async {
                let found = self
                    .run(&["realpath", "--", &self.root], FS_TIMEOUT_SECONDS)
                    .await?;
                let resolved_root = found.stdout.trim();
                let usable = found.exit_code == 0_i32 && !resolved_root.is_empty();
                Ok::<String, HarnessError>(
                    if usable { resolved_root } else { &self.root }.to_owned(),
                )
            })
            .await?;
        if resolved != root
            && !resolved
                .strip_prefix(root.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
        {
            return Err(BackendError::OutsideWorkspace {
                path: path.to_owned(),
            });
        }
        Ok(real)
    }

    async fn run(&self, argv: &[&str], timeout_seconds: u64) -> Result<CompletedRun, HarnessError> {
        let command = SandboxCommand {
            argv: argv.iter().map(|part| (*part).to_owned()).collect(),
            env: BTreeMap::new(),
            cwd: None,
        };
        let timeout = Duration::from_secs(timeout_seconds);
        self.sandbox.run(&command, timeout, RUN_CAP_BYTES).await
    }

    async fn read_bytes(&self, path: &str) -> Result<Vec<u8>, BackendError> {
        let real = self.to_confined(path).await?;
        self.sandbox
            .read(&real, READ_CAP_BYTES)
            .await
            .map_err(BackendError::Sandbox)
    }

    async fn write_bytes(&self, path: &str, data: &[u8]) -> Result<(), BackendError> {
        let real = self.to_confined(path).await?;
        self.sandbox
            .write(&real, data)
            .await
            .map_err(BackendError::Sandbox)
    }
}

// posixpath.normpath of an absolute path: ".." at the root stays at the root
fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            name => parts.push(name),
        }
    }
    format!("/{}", parts.join("/"))
}
