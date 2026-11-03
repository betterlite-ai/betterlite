use std::path::{Component, Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum CategoryPathError {
    #[error("category file path '{path}' is outside the allowed data roots ({roots})")]
    OutsideRoots { path: String, roots: String },
}

pub fn assert_within_data_roots(path: &Path, roots: &[PathBuf]) -> Result<(), CategoryPathError> {
    let normalized = normalize(path);
    if roots
        .iter()
        .map(|root| normalize(root))
        .any(|root| normalized.starts_with(root))
    {
        return Ok(());
    }
    Err(CategoryPathError::OutsideRoots {
        path: path.display().to_string(),
        roots: roots
            .iter()
            .map(|root| root.display().to_string())
            .collect::<Vec<_>>()
            .join(", "),
    })
}

pub fn resolve_category_file_path(
    file_path: &Path,
    roots: &[PathBuf],
    data_dir: &Path,
    allow_external: bool,
) -> Result<PathBuf, CategoryPathError> {
    if file_path.is_absolute() || file_path.exists() {
        if allow_external {
            tracing::warn!(path = %file_path.display(), "LITELLM_CONTENT_FILTER_ALLOW_EXTERNAL_PATHS is set; bypassing category path jail");
            return Ok(file_path.to_path_buf());
        }
        assert_within_data_roots(file_path, roots)?;
        return Ok(file_path.to_path_buf());
    }
    let parts: Vec<_> = file_path.components().collect();
    for start in 0..parts.len() {
        for root in roots {
            let Some(suffix_parts) = parts.get(start..) else {
                continue;
            };
            let suffix: PathBuf = suffix_parts.iter().collect();
            let Some(candidate) = joined_candidate(root, &suffix, allow_external) else {
                continue;
            };
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }
    if allow_external {
        return Ok(file_path.to_path_buf());
    }
    let data_relative = data_dir.join(file_path);
    assert_within_data_roots(&data_relative, roots)?;
    Ok(file_path.to_path_buf())
}

fn joined_candidate(root: &Path, suffix: &Path, allow_external: bool) -> Option<PathBuf> {
    if allow_external {
        return Some(root.join(suffix));
    }
    safe_join(root, suffix)
}

fn safe_join(root: &Path, suffix: &Path) -> Option<PathBuf> {
    if suffix
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return None;
    }
    Some(root.join(suffix))
}

fn normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(Component::RootDir.as_os_str()),
            Component::Normal(part) => normalized.push(part),
        }
    }
    normalized
}
