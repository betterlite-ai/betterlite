use std::fs::{OpenOptions, Permissions};
use std::io::{self, Write as _};
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

use thiserror::Error;

pub(super) const PEM_PREFIX: &str = "-----BEGIN";
const PEM_DIRECTORY_PREFIX: &str = "betterlite-billing-mtls-";
const PEM_DIRECTORY_MODE: u32 = 0o700;
const PEM_FILE_MODE: u32 = 0o600;

#[derive(Debug, Error)]
pub(super) enum CredentialError {
    #[error("could not create a temporary directory: no candidates were configured")]
    NoTemporaryDirectory,
    #[error("could not write inline certificate content to disk at {}: {source}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CredentialPaths {
    pub(super) client_cert: PathBuf,
    pub(super) client_key: PathBuf,
    pub(super) ca_cert: Option<PathBuf>,
}

pub(super) fn resolve_paths(
    temp_root_candidates: &[PathBuf],
    client_cert: &str,
    client_key: &str,
    ca_cert: Option<&str>,
) -> Result<CredentialPaths, CredentialError> {
    let inline = [Some(client_cert), Some(client_key), ca_cert]
        .into_iter()
        .flatten()
        .any(|value| value.trim_start().starts_with(PEM_PREFIX));
    if !inline {
        return Ok(CredentialPaths {
            client_cert: PathBuf::from(client_cert),
            client_key: PathBuf::from(client_key),
            ca_cert: ca_cert.map(PathBuf::from),
        });
    }
    // the directory is kept for the life of the process, since the exporter reads the files by path
    let directory = create_pem_directory(temp_root_candidates)?;
    Ok(CredentialPaths {
        client_cert: credential_path(&directory, "client.crt", client_cert)?,
        client_key: credential_path(&directory, "client.key", client_key)?,
        ca_cert: ca_cert
            .map(|value| credential_path(&directory, "ca.crt", value))
            .transpose()?,
    })
}

// Mirrors tempfile.mkdtemp trying every tempfile.gettempdir candidate, not just the first.
fn create_pem_directory(candidates: &[PathBuf]) -> Result<PathBuf, CredentialError> {
    let mut last_error = None;
    for candidate in candidates {
        let attempt = tempfile::Builder::new()
            .prefix(PEM_DIRECTORY_PREFIX)
            .permissions(Permissions::from_mode(PEM_DIRECTORY_MODE))
            .tempdir_in(candidate);
        match attempt {
            Ok(directory) => return Ok(directory.keep()),
            Err(source) => {
                last_error = Some(CredentialError::Write {
                    path: candidate.clone(),
                    source,
                });
            }
        }
    }
    let Some(error) = last_error else {
        return Err(CredentialError::NoTemporaryDirectory);
    };
    Err(error)
}

fn credential_path(
    directory: &Path,
    filename: &str,
    value: &str,
) -> Result<PathBuf, CredentialError> {
    if !value.trim_start().starts_with(PEM_PREFIX) {
        return Ok(PathBuf::from(value));
    }
    let path = directory.join(filename);
    write_pem(&path, value).map_err(|source| CredentialError::Write {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

fn write_pem(path: &Path, pem: &str) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(PEM_FILE_MODE)
        .open(path)?;
    file.write_all(pem.as_bytes())?;
    if !pem.ends_with('\n') {
        file.write_all(b"\n")?;
    }
    Ok(())
}
