use std::io;
use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum OidcError {
    #[error(
        "oidc/file path must be absolute. Use the format 'oidc/file//var/run/secrets/<name>' (note the leading slash after 'oidc/file/')."
    )]
    RelativePath,
    #[error(
        "oidc/file path is outside the allowed credential directories. Set LITELLM_OIDC_ALLOWED_CREDENTIAL_DIRS to extend the allowlist."
    )]
    OutsideAllowedDirs,
    #[error("oidc path is longer than {limit} bytes")]
    PathTooLong { limit: usize },
    #[error("oidc path {} could not be resolved", path.display())]
    Unresolvable {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}
