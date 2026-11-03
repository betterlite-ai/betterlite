use std::{
    io,
    path::{Path, PathBuf},
};

#[derive(Debug, thiserror::Error)]
pub enum CategoryPathError {
    #[error("cannot resolve category path {path}: {source}")]
    Resolve {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("category file path '{path}' is outside the allowed data roots ({roots})")]
    OutsideRoots { path: String, roots: String },
}

pub fn assert_within_data_roots(path: &Path, roots: &[PathBuf]) -> Result<(), CategoryPathError> {
    let normalized = resolve_real(path)?;
    for root in roots {
        if normalized.starts_with(resolve_real(root)?) {
            return Ok(());
        }
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
            let Some(candidate) = joined_candidate(root, &suffix, allow_external)? else {
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

fn joined_candidate(
    root: &Path,
    suffix: &Path,
    allow_external: bool,
) -> Result<Option<PathBuf>, CategoryPathError> {
    if allow_external {
        return Ok(Some(root.join(suffix)));
    }
    let base = resolve_real(root)?;
    let candidate = resolve_real(&base.join(suffix))?;
    Ok(candidate.starts_with(&base).then_some(candidate))
}

fn resolve_real(path: &Path) -> Result<PathBuf, CategoryPathError> {
    soft_canonicalize::soft_canonicalize(path).map_err(|source| CategoryPathError::Resolve {
        path: path.to_path_buf(),
        source,
    })
}
