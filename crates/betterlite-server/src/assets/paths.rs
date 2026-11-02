use std::path::{Component, Path, PathBuf, absolute};

use soft_canonicalize::soft_canonicalize;

#[derive(Debug, thiserror::Error)]
pub enum PathError {
    #[error("Path contains null byte")]
    NullByte,
    #[error("Path {resolved:?} escapes base directory {base:?}")]
    Escaped { resolved: PathBuf, base: PathBuf },
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
    let base = realpath_non_strict(base_dir);
    let joined = push_parts(&base, parts);
    let resolved = realpath_non_strict(&joined);
    if resolved.starts_with(&base) {
        Ok(resolved)
    } else {
        Err(PathError::Escaped { resolved, base })
    }
}

#[must_use]
pub fn try_safe_join(base_dir: &Path, parts: &[&str]) -> Option<PathBuf> {
    let result = safe_join(base_dir, parts);
    if result.is_err() {
        return None;
    }
    result.ok()
}

#[must_use]
pub fn is_within(path: &Path, base_dir: &Path) -> bool {
    let base = realpath_non_strict(base_dir);
    let resolved = realpath_non_strict(path);
    resolved.starts_with(&base)
}

// Mirrors `os.path.realpath(path, strict=False)`: a failed component stays literal, but a later `..` still pops it and a later component still tries to resolve, as in CPython's `_joinrealpath`.
fn realpath_non_strict(path: &Path) -> PathBuf {
    let Ok(absolute_path) = absolute(path) else {
        return path.to_path_buf();
    };
    if let Ok(resolved) = soft_canonicalize(&absolute_path) {
        return resolved;
    }
    let mut resolved = PathBuf::new();
    for component in absolute_path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(
                    resolved.components().next_back(),
                    Some(Component::Normal(_))
                ) {
                    resolved.pop();
                }
            }
            Component::Prefix(_) | Component::RootDir => resolved.push(component),
            Component::Normal(name) => {
                let candidate = resolved.join(name);
                resolved = soft_canonicalize(&candidate).unwrap_or(candidate);
            }
        }
    }
    resolved
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
