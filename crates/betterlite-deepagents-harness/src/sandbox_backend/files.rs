use std::future::Future;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_harness::sandbox::{CompletedRun, Sandbox};

use crate::sandbox_backend::errors::BackendError;
use crate::sandbox_backend::protocol::{
    DeleteResult, EditResult, Edited, Encoding, FileData, FileDownloadResponse, FileInfo, LsResult,
    ReadResult, Replacement, Reply, TransferError,
};
use crate::sandbox_backend::read_window::slice_read;
use crate::sandbox_backend::replace::replace_occurrences;
use crate::sandbox_backend::transfer::transfer_error;
use crate::sandbox_backend::{
    DELETE_SCRIPT, EXIT_NOT_DIR, EXIT_NOT_FOUND, FS_TIMEOUT_SECONDS, LS_SCRIPT, READ_ONLY_ERROR,
    SandboxBackend,
};

pub(crate) async fn list<S>(backend: &SandboxBackend<S>, path: &str) -> LsResult
where
    S: Sandbox,
{
    let (real, done) = match script_at(backend, LS_SCRIPT, path).await {
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

pub(crate) async fn download<S>(backend: &SandboxBackend<S>, path: &str) -> FileDownloadResponse
where
    S: Sandbox,
{
    let content = backend.read_bytes(path).await;
    FileDownloadResponse {
        path: path.to_owned(),
        content: content.map_err(|error| transfer_error(error, |_| TransferError::FileNotFound)),
    }
}

// write, edit and delete run only through here, so a read-only session never touches a file
pub(crate) async fn when_writable<S, T, F>(backend: &SandboxBackend<S>, change: F) -> Reply<T>
where
    S: Sandbox,
    F: Future<Output = Reply<T>>,
{
    if backend.permissions.writable {
        change.await
    } else {
        Reply::Error(READ_ONLY_ERROR.to_owned())
    }
}

pub(crate) async fn edit<S>(
    backend: &SandboxBackend<S>,
    file_path: &str,
    change: Replacement<'_>,
) -> EditResult
where
    S: Sandbox,
{
    // LiteLLM decodes with replacement and would write the mangled bytes back
    let text = match backend.read_bytes(file_path).await.map(String::from_utf8) {
        Ok(Ok(text)) => text,
        Ok(Err(_)) => {
            return Reply::Error(format!(
                "Error editing file '{file_path}': the file is not UTF-8 text"
            ));
        }
        Err(BackendError::Sandbox(_)) => {
            return Reply::Error(format!("Error: File '{file_path}' not found"));
        }
        Err(error) => return Reply::Error(format!("Error editing file '{file_path}': {error}")),
    };
    let normalized = text.replace("\r\n", "\n");
    let old = change.old.replace("\r\n", "\n");
    let new = change.new.replace("\r\n", "\n");
    let replaced = match replace_occurrences(&normalized, &old, &new, change.replace_all) {
        Ok(replaced) => replaced,
        Err(error) => return Reply::Error(error.to_string()),
    };
    let written = backend.write_bytes(file_path, replaced.content.as_bytes());
    written.await.map_or_else(
        |error| Reply::Error(format!("Error editing file '{file_path}': {error}")),
        |()| {
            Reply::Done(Edited {
                path: file_path.to_owned(),
                occurrences: replaced.occurrences,
            })
        },
    )
}

pub(crate) async fn delete<S>(backend: &SandboxBackend<S>, file_path: &str) -> DeleteResult
where
    S: Sandbox,
{
    // to_confined hands back to_real's path, so the root check needs no command
    if backend.to_real(file_path) == backend.root {
        return Reply::Error("Error: refusing to delete the workspace root".to_owned());
    }
    let done = match script_at(backend, DELETE_SCRIPT, file_path).await {
        Ok((_, done)) => done,
        Err(error) => return Reply::Error(format!("Error deleting '{file_path}': {error}")),
    };
    if done.exit_code == EXIT_NOT_FOUND {
        return Reply::Error(format!("Error: '{file_path}' not found"));
    }
    if done.exit_code != 0_i32 {
        let stderr = done.stderr.trim();
        return Reply::Error(format!("Error deleting '{file_path}': {stderr}"));
    }
    Reply::Done(file_path.to_owned())
}

async fn script_at<S>(
    backend: &SandboxBackend<S>,
    script: &str,
    path: &str,
) -> Result<(String, CompletedRun), BackendError>
where
    S: Sandbox,
{
    let real = backend.to_confined(path).await?;
    let done = backend
        .run(&["sh", "-c", script, "sh", &real], FS_TIMEOUT_SECONDS)
        .await?;
    Ok((real, done))
}
