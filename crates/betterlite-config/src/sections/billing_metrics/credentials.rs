use std::fs::{File, OpenOptions, Permissions};
use std::io::{self, Write as _};
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

use thiserror::Error;

use super::{CA_CERT_ENV, CLIENT_CERT_ENV, CLIENT_KEY_ENV};

pub(super) const PEM_PREFIX: &str = "-----BEGIN";
const PEM_DIRECTORY_PREFIX: &str = "betterlite-billing-mtls-";
const PEM_DIRECTORY_MODE: u32 = 0o700;
const PEM_FILE_MODE: u32 = 0o600;

#[derive(Debug, Error)]
pub(super) enum CredentialError {
    #[error("could not create a private directory for inline certificate content under {}: {source}", root.display())]
    Directory {
        root: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not write inline certificate content to {}: {source}", path.display())]
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

impl CredentialPaths {
    pub(super) fn unreadable(&self) -> Vec<&'static str> {
        [
            (CLIENT_CERT_ENV, Some(&self.client_cert)),
            (CLIENT_KEY_ENV, Some(&self.client_key)),
            (CA_CERT_ENV, self.ca_cert.as_ref()),
        ]
        .into_iter()
        .filter_map(|(variable, path)| {
            path.filter(|candidate| !(candidate.is_file() && File::open(candidate).is_ok()))
                .map(|_unreadable| variable)
        })
        .collect()
    }
}

pub(super) fn resolve_paths(
    temp_root: &Path,
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
    let directory = tempfile::Builder::new()
        .prefix(PEM_DIRECTORY_PREFIX)
        .permissions(Permissions::from_mode(PEM_DIRECTORY_MODE))
        .tempdir_in(temp_root)
        .map_err(|source| CredentialError::Directory {
            root: temp_root.to_path_buf(),
            source,
        })?
        .keep();
    Ok(CredentialPaths {
        client_cert: credential_path(&directory, "client.crt", client_cert)?,
        client_key: credential_path(&directory, "client.key", client_key)?,
        ca_cert: ca_cert
            .map(|value| credential_path(&directory, "ca.crt", value))
            .transpose()?,
    })
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
