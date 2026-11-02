use std::io;
use std::path::{Component, Path, PathBuf, absolute};

use soft_canonicalize::soft_canonicalize;

#[derive(Debug, thiserror::Error)]
pub enum PathError {
    #[error("Path contains null byte")]
    NullByte,
    #[error("Path {resolved:?} escapes base directory {base:?}")]
    Escaped { resolved: PathBuf, base: PathBuf },
    #[error("could not resolve path")]
    Io(#[source] io::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum FilenameError {
    #[error("Filename contains null byte")]
    NullByte,
    #[error("Empty or unsafe filename")]
    EmptyOrUnsafe,
}

fn push_parts(base: &Path, parts: &[&str]) -> PathBuf {
    let mut joined = base.to_path_buf();
    for part in parts {
        joined.push(part);
    }
    joined
}

fn has_null_byte(parts: &[&str]) -> bool {
    for part in parts {
        if part.contains('\0') {
            return true;
        }
    }
    false
}

pub fn safe_join(base_dir: &Path, parts: &[&str]) -> Result<PathBuf, PathError> {
    if has_null_byte(parts) {
        return Err(PathError::NullByte);
    }
    let base = soft_canonicalize(base_dir).map_err(PathError::Io)?;
    let joined = push_parts(&base, parts);
    let resolved = soft_canonicalize(&joined).map_err(PathError::Io)?;
    if resolved.starts_with(&base) {
        Ok(resolved)
    } else {
        Err(PathError::Escaped { resolved, base })
    }
}

pub fn try_safe_join(base_dir: &Path, parts: &[&str]) -> Result<Option<PathBuf>, io::Error> {
    match safe_join(base_dir, parts) {
        Ok(path) => Ok(Some(path)),
        Err(PathError::NullByte | PathError::Escaped { .. }) => Ok(None),
        Err(PathError::Io(source)) => Err(source),
    }
}

pub fn is_within(path: &Path, base_dir: &Path) -> Result<bool, io::Error> {
    let base = soft_canonicalize(base_dir)?;
    let resolved = soft_canonicalize(path)?;
    Ok(resolved.starts_with(&base))
}

#[must_use]
pub fn join_within(base_dir: &Path, parts: &[&str]) -> Option<PathBuf> {
    if has_null_byte(parts) {
        return None;
    }
    let base = lexically_normalize(&absolute(base_dir).ok()?);
    let joined = push_parts(&base, parts);
    let normalized = lexically_normalize(&joined);
    if normalized == base || !normalized.starts_with(&base) {
        return None;
    }
    Some(normalized)
}

pub fn safe_filename(filename: &str) -> Result<String, FilenameError> {
    if filename.contains('\0') {
        return Err(FilenameError::NullByte);
    }
    let normalized = filename.replace('\\', "/");
    let name = normalized.rsplit('/').next().unwrap_or_default();
    if name.is_empty() || name == "." || name == ".." {
        return Err(FilenameError::EmptyOrUnsafe);
    }
    Ok(name.to_owned())
}

// `base_dir` is always already absolute here, so a leading ParentDir never pops past RootDir.
fn lexically_normalize(path: &Path) -> PathBuf {
    let mut stack = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(stack.last(), Some(Component::Normal(_))) {
                    stack.pop();
                }
            }
            other @ (Component::Prefix(_) | Component::RootDir | Component::Normal(_)) => {
                stack.push(other);
            }
        }
    }
    stack.into_iter().collect()
}

mod tests;
