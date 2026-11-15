use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use betterlite_config::loader::EnvironmentLookup;

use super::error::DebugError;

pub const SESSION_ID_ENV: &str = "CLAUDE_CODE_SESSION_ID";
const TRANSCRIPT_EXTENSION: &str = "jsonl";
const UUID_GROUP_LENGTHS: [usize; 5] = [8, 4, 4, 4, 12];

pub fn detect_claude_session_id(
    environment: &dyn EnvironmentLookup,
    claude_dir: &Path,
) -> Result<Option<String>, DebugError> {
    if let Some(explicit) = environment
        .lookup(SESSION_ID_ENV)
        .filter(|value| !value.is_empty())
    {
        return Ok(Some(explicit));
    }
    let mut transcripts = Vec::new();
    for project in list_directory(&claude_dir.join("projects"))? {
        if project.is_dir() {
            transcripts.extend(transcripts_in(&project)?);
        }
    }
    Ok(transcripts
        .into_iter()
        .max_by_key(|(modified, _stem)| *modified)
        .map(|(_modified, stem)| stem))
}

fn transcripts_in(project: &Path) -> Result<Vec<(SystemTime, String)>, DebugError> {
    let mut transcripts = Vec::new();
    for path in list_directory(project)? {
        let Some(stem) = transcript_stem(&path) else {
            continue;
        };
        let modified = fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .map_err(|source| DebugError::TranscriptScan { path, source })?;
        transcripts.push((modified, stem));
    }
    Ok(transcripts)
}

fn transcript_stem(path: &Path) -> Option<String> {
    if path.extension()? != TRANSCRIPT_EXTENSION {
        return None;
    }
    let stem = path.file_stem()?.to_str()?;
    is_session_uuid(stem).then(|| stem.to_owned())
}

fn is_session_uuid(stem: &str) -> bool {
    let groups: Vec<&str> = stem.split('-').collect();
    groups.len() == UUID_GROUP_LENGTHS.len()
        && groups
            .iter()
            .zip(UUID_GROUP_LENGTHS)
            .all(|(group, length)| {
                group.len() == length
                    && group
                        .chars()
                        .all(|character| matches!(character, '0'..='9' | 'a'..='f'))
            })
}

fn list_directory(directory: &Path) -> Result<Vec<PathBuf>, DebugError> {
    let scan_error = |source: io::Error| DebugError::TranscriptScan {
        path: directory.to_owned(),
        source,
    };
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(scan_error(source)),
    };
    entries
        .map(|entry| entry.map(|found| found.path()).map_err(scan_error))
        .collect()
}

mod tests;
