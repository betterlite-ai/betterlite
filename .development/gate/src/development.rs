use std::path::{Path, PathBuf};

use crate::backlog::read_backlog;
use crate::error::GateError;
use crate::git::git;
use crate::spec::backlog::Backlog;
use crate::spec::catalog::Catalog;
use crate::spec::task::Task;
use crate::yaml::read_yaml;

pub(crate) struct Development {
    pub(crate) root: PathBuf,
    pub(crate) catalog: Catalog,
    pub(crate) backlog: Backlog,
}

impl Development {
    pub(crate) fn open(directory: &Path) -> Result<Self, GateError> {
        let root = PathBuf::from(git(directory, &["rev-parse", "--show-toplevel"])?.trim());
        let catalog = read_yaml(&root.join(".development/checks.yaml"))?;
        let (backlog, problems) = read_backlog(&root.join(".development"))?;
        if !problems.is_empty() {
            return Err(GateError::Backlog(problems.join("; ")));
        }
        Ok(Self {
            root,
            catalog,
            backlog,
        })
    }

    pub(crate) fn task(&self, id: &str) -> Result<Task, GateError> {
        let entry = self
            .backlog
            .tasks
            .iter()
            .find(|candidate| candidate.id == id)
            .ok_or_else(|| GateError::UnknownTask(id.to_owned()))?;
        let file = entry
            .file
            .as_ref()
            .ok_or_else(|| GateError::NoTaskFile(id.to_owned()))?;
        read_yaml(&self.root.join(".development").join(file))
    }
}
