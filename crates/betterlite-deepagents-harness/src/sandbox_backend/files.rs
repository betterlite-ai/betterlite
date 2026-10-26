use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_harness::sandbox::{CompletedRun, Sandbox};

use crate::sandbox_backend::errors::BackendError;
use crate::sandbox_backend::protocol::{
    DeleteResult, EditResult, Edited, Encoding, FileData, FileInfo, LsResult, ReadResult, Reply,
    WriteResult,
};
use crate::sandbox_backend::read_window::slice_read;
use crate::sandbox_backend::replace::replace_occurrences;
use crate::sandbox_backend::{
    DELETE_SCRIPT, EXIT_NOT_DIR, EXIT_NOT_FOUND, FS_TIMEOUT_SECONDS, LS_SCRIPT, READ_ONLY_ERROR,
    SandboxBackend,
};

pub(crate) async fn list<S>(backend: &SandboxBackend<S>, path: &str) -> LsResult
where
    S: Sandbox,
{
    let (real, done) = match ls_run(backend, path).await {
        Ok(listed) => listed,
        Err(error) => return Reply::Error(format!("Path '{path}': {error}")),
    };
    match done.exit_code {
        0_i32 => {}
        EXIT_NOT_FOUND => return Reply::Error(format!("Path '{path}': path_not_found")),
        EXIT_NOT_DIR => return Reply::Error(format!("Path '{path}': not_a_directory")),
        _ => {
            let stderr = done.stderr.trim();
            let reason = if stderr.is_empty() {
                "ls failed"
            } else {
                stderr
            };
            return Reply::Error(format!("Path '{path}': {reason}"));
        }
    }
    let directory = backend.to_virtual(&real);
    let base = directory.trim_end_matches('/');
    let mut entries: Vec<FileInfo> = done
        .stdout
        .lines()
        .filter_map(|line| line.split_once('/'))
        .filter(|(_, name)| !name.is_empty())
        .map(|(kind, name)| FileInfo {
            path: format!("{base}/{name}{}", if kind == "d" { "/" } else { "" }),
            is_dir: kind == "d",
        })
        .collect();
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Reply::Done(entries)
}

pub(crate) async fn read<S>(
    backend: &SandboxBackend<S>,
    file_path: &str,
    offset: usize,
    limit: usize,
) -> ReadResult
where
    S: Sandbox,
{
    let data = match backend.read_bytes(file_path).await {
        Ok(data) => data,
        Err(BackendError::Sandbox(_)) => {
            return ReadResult::Error(format!("File '{file_path}' not found"));
        }
        Err(error) => {
            return ReadResult::Error(format!("Error reading file '{file_path}': {error}"));
        }
    };
    match String::from_utf8(data) {
        Ok(text) => slice_read(text, offset, limit),
        Err(binary) => ReadResult::Whole(FileData {
            content: STANDARD.encode(binary.into_bytes()),
            encoding: Encoding::Base64,
        }),
    }
}

pub(crate) async fn write<S>(
    backend: &SandboxBackend<S>,
    file_path: &str,
    content: &str,
) -> WriteResult
where
    S: Sandbox,
{
    if !backend.permissions.writable {
        return Reply::Error(READ_ONLY_ERROR.to_owned());
    }
    match backend.write_bytes(file_path, content.as_bytes()).await {
        Ok(()) => Reply::Done(file_path.to_owned()),
        Err(error) => Reply::Error(format!("Error writing file '{file_path}': {error}")),
    }
}

pub(crate) async fn edit<S>(
    backend: &SandboxBackend<S>,
    file_path: &str,
    old: &str,
    new: &str,
    replace_all: bool,
) -> EditResult
where
    S: Sandbox,
{
    if !backend.permissions.writable {
        return Reply::Error(READ_ONLY_ERROR.to_owned());
    }
    let data = match backend.read_bytes(file_path).await {
        Ok(data) => data,
        Err(BackendError::Sandbox(_)) => {
            return Reply::Error(format!("Error: File '{file_path}' not found"));
        }
        Err(error) => return Reply::Error(format!("Error editing file '{file_path}': {error}")),
    };
    // LiteLLM decodes with replacement and would write the mangled bytes back
    let Ok(text) = String::from_utf8(data) else {
        return Reply::Error(format!(
            "Error editing file '{file_path}': the file is not UTF-8 text"
        ));
    };
    let replaced = match replace_occurrences(
        &text.replace("\r\n", "\n"),
        &old.replace("\r\n", "\n"),
        &new.replace("\r\n", "\n"),
        replace_all,
    ) {
        Ok(replaced) => replaced,
        Err(error) => return Reply::Error(error.to_string()),
    };
    match backend
        .write_bytes(file_path, replaced.content.as_bytes())
        .await
    {
        Ok(()) => Reply::Done(Edited {
            path: file_path.to_owned(),
            occurrences: replaced.occurrences,
        }),
        Err(error) => Reply::Error(format!("Error editing file '{file_path}': {error}")),
    }
}

pub(crate) async fn delete<S>(backend: &SandboxBackend<S>, file_path: &str) -> DeleteResult
where
    S: Sandbox,
{
    if !backend.permissions.writable {
        return Reply::Error(READ_ONLY_ERROR.to_owned());
    }
    let done = match delete_run(backend, file_path).await {
        Ok(Some(done)) => done,
        Ok(None) => return Reply::Error("Error: refusing to delete the workspace root".to_owned()),
        Err(error) => return Reply::Error(format!("Error deleting '{file_path}': {error}")),
    };
    match done.exit_code {
        0_i32 => Reply::Done(file_path.to_owned()),
        EXIT_NOT_FOUND => Reply::Error(format!("Error: '{file_path}' not found")),
        _ => Reply::Error(format!(
            "Error deleting '{file_path}': {}",
            done.stderr.trim()
        )),
    }
}

async fn ls_run<S>(
    backend: &SandboxBackend<S>,
    path: &str,
) -> Result<(String, CompletedRun), BackendError>
where
    S: Sandbox,
{
    let real = backend.to_confined(path).await?;
    let done = backend
        .run(&["sh", "-c", LS_SCRIPT, "sh", &real], FS_TIMEOUT_SECONDS)
        .await?;
    Ok((real, done))
}

async fn delete_run<S>(
    backend: &SandboxBackend<S>,
    path: &str,
) -> Result<Option<CompletedRun>, BackendError>
where
    S: Sandbox,
{
    let real = backend.to_confined(path).await?;
    if real == backend.root {
        return Ok(None);
    }
    let done = backend
        .run(
            &["sh", "-c", DELETE_SCRIPT, "sh", &real],
            FS_TIMEOUT_SECONDS,
        )
        .await?;
    Ok(Some(done))
}
