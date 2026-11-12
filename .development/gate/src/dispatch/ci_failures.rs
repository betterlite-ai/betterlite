mod tests;

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use crate::error::GateError;
use crate::git::git;

// early-pr.sh writes the head it pushed on the first line, then the failure lines of CI on it
pub(super) fn take(journal: &Path, worktree: &Path) -> Result<Vec<String>, GateError> {
    let path = journal.join("ci-failures");
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(GateError::Read { path, source }),
    };
    fs::remove_file(&path).map_err(|source| GateError::Write {
        path: path.clone(),
        source,
    })?;
    let head = git(worktree, &["rev-parse", "HEAD"])?;
    let mut lines = text.lines();
    if lines.next() != Some(head.trim()) {
        return Ok(Vec::new());
    }
    Ok(lines.map(str::to_owned).collect())
}
