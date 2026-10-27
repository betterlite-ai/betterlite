use std::fs;
use std::mem;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rustix::process::Signal;
use tempfile::{Builder, TempDir};

use super::error::SandboxError;
use super::files::{path_string, real_path};
use super::process::OwnedChild;
use crate::errors::HarnessError;

#[derive(Default)]
pub(super) struct State {
    pub(super) closed: bool,
    pub(super) processes: Vec<Arc<OwnedChild>>,
    pub(super) tempdirs: Vec<TempDir>,
}

impl State {
    pub(super) fn resolve(&self, root: &str, path: &str) -> Result<PathBuf, HarnessError> {
        if self.closed {
            return Err(HarnessError::from(SandboxError::Closed));
        }
        let joined = Path::new(root).join(path);
        let resolved = real_path(&joined).map_err(|source| HarnessError::Sandbox { source })?;
        if !resolved.starts_with(root)
            && !self
                .tempdirs
                .iter()
                .any(|dir| resolved.starts_with(dir.path()))
        {
            return Err(HarnessError::from(SandboxError::Escape(joined)));
        }
        Ok(resolved)
    }

    pub(super) fn create_tempdir(&mut self, root: &Path) -> Result<String, HarnessError> {
        if self.closed {
            return Err(HarnessError::from(SandboxError::Closed));
        }
        let directory = Builder::new()
            .prefix("litellm-harness-")
            .tempdir_in(root)
            .map_err(|source| HarnessError::Sandbox { source })?;
        let canonical = fs::canonicalize(directory.path())
            .map_err(|source| HarnessError::Sandbox { source })?;
        let name = path_string(&canonical)?;
        self.tempdirs.push(directory);
        Ok(name)
    }

    pub(super) fn take(&mut self) -> Self {
        self.closed = true;
        Self {
            closed: true,
            processes: mem::take(&mut self.processes),
            tempdirs: mem::take(&mut self.tempdirs),
        }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        for process in &self.processes {
            drop(process.signal(Signal::KILL));
        }
    }
}
