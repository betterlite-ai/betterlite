use std::env;

use super::error::DockerSandboxError;
use crate::errors::HarnessError;

pub(super) const DEFAULT_WORKDIR: &str = "/workspace";

pub(super) fn normalize_path(path: &str) -> String {
    if path.is_empty() {
        return ".".to_owned();
    }
    // posixpath.normpath keeps exactly two leading slashes and folds three or more into one
    let slash_count = path.bytes().take_while(|byte| *byte == b'/').count();
    let leading = match slash_count {
        0 => 0_usize,
        2 => 2_usize,
        _ => 1_usize,
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." if leading == 0 && parts.is_empty() => parts.push(part),
            ".." if parts.last().is_some_and(|last| *last == "..") => parts.push(part),
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    let normalized = format!("{}{}", "/".repeat(leading), parts.join("/"));
    if normalized.is_empty() {
        ".".to_owned()
    } else {
        normalized
    }
}

pub(super) fn resolve_mounts(
    mounts: Vec<(String, String)>,
) -> Result<Vec<(String, String)>, HarnessError> {
    let mut resolved: Vec<(String, String)> = Vec::new();
    for (host, container) in mounts {
        let absolute = absolute_host_path(&host)?;
        match resolved.iter_mut().find(|entry| entry.0 == absolute) {
            Some(entry) => entry.1 = container,
            None => resolved.push((absolute, container)),
        }
    }
    Ok(resolved)
}

fn absolute_host_path(host: &str) -> Result<String, HarnessError> {
    if host.starts_with('/') {
        return Ok(normalize_path(host));
    }
    let current = env::current_dir().map_err(|source| HarnessError::Sandbox { source })?;
    let base = current
        .to_str()
        .ok_or_else(|| DockerSandboxError::PathEncoding {
            path: current.clone(),
        })?;
    let separator = if base.ends_with('/') { "" } else { "/" };
    Ok(normalize_path(&format!("{base}{separator}{host}")))
}
