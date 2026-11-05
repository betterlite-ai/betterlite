use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use soft_canonicalize::soft_canonicalize;

use crate::oidc::error::OidcError;

const ALLOWED_DIRS_VARIABLE: &str = "LITELLM_OIDC_ALLOWED_CREDENTIAL_DIRS";
const DEFAULT_ALLOWED_DIRS: [&str; 2] = ["/var/run/secrets", "/run/secrets"];
// the longest path the kernel accepts, so nothing longer can name a file
const MAX_PATH_BYTES: usize = 4096;

fn real_path(path: &str) -> Result<PathBuf, OidcError> {
    if path.len() > MAX_PATH_BYTES {
        return Err(OidcError::PathTooLong {
            limit: MAX_PATH_BYTES,
        });
    }
    soft_canonicalize(path).map_err(|source| OidcError::Unresolvable {
        path: PathBuf::from(path),
        source,
    })
}

pub fn oidc_allowed_dirs(
    environment: &BTreeMap<String, String>,
) -> Result<Vec<PathBuf>, OidcError> {
    let configured = environment
        .get(ALLOWED_DIRS_VARIABLE)
        .filter(|list| !list.is_empty());
    let directories: Vec<&str> = configured.map_or_else(
        || DEFAULT_ALLOWED_DIRS.to_vec(),
        |list| {
            list.split(',')
                .map(str::trim)
                .filter(|directory| !directory.is_empty())
                .collect()
        },
    );
    directories.into_iter().map(real_path).collect()
}

pub fn resolve_oidc_file_path(
    requested: &str,
    environment: &BTreeMap<String, String>,
) -> Result<PathBuf, OidcError> {
    if !Path::new(requested).is_absolute() {
        return Err(OidcError::RelativePath);
    }
    let resolved = real_path(requested)?;
    let allowed = oidc_allowed_dirs(environment)?;
    if allowed
        .iter()
        .any(|directory| resolved.starts_with(directory))
    {
        Ok(resolved)
    } else {
        Err(OidcError::OutsideAllowedDirs)
    }
}
