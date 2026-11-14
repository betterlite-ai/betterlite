use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use super::error::DebugError;
use crate::settings_backup::secure::secure_create;

pub fn write_report(
    report: &str,
    session_id: &str,
    report_dir: &Path,
) -> Result<PathBuf, DebugError> {
    if session_id.contains(['/', '\\']) {
        return Err(DebugError::SessionIdNotFileName {
            session_id: session_id.to_owned(),
        });
    }
    fs::create_dir_all(report_dir).map_err(|source| DebugError::ReportDirectory {
        path: report_dir.to_owned(),
        source,
    })?;
    let path = report_dir.join(format!("claude-{session_id}.md"));
    let mut file = secure_create(&path).map_err(|source| DebugError::ReportCreate {
        path: path.clone(),
        source,
    })?;
    file.write_all(report.as_bytes())
        .map_err(|source| DebugError::ReportWrite {
            path: path.clone(),
            source,
        })?;
    Ok(path)
}

mod tests;
