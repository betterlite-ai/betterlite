use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Component, Path};

use thiserror::Error;

use crate::OracleError;

const MAX_FILES: usize = 16;
const MAX_BODY_BYTES: usize = 64 * 1024;
const MAX_TOTAL_BYTES: usize = 256 * 1024;
const MAX_DESTINATION_BYTES: usize = 512;
const MAX_COMPONENTS: usize = 8;

#[derive(Debug, Error)]
pub enum ConfigFixtureError {
    #[error("config_files[{destination}] is not a relative file destination: {reason}")]
    UnsafeDestination {
        destination: String,
        reason: &'static str,
    },
    #[error("config_files[{destination}] is {bytes} bytes, past the limit of {max}")]
    DestinationTooLong {
        destination: String,
        bytes: usize,
        max: usize,
    },
    #[error("config_files declares {count} files, past the limit of {max}")]
    TooManyFiles { count: usize, max: usize },
    #[error("config_files[{destination}] is {bytes} bytes, past the limit of {max}")]
    BodyTooLarge {
        destination: String,
        bytes: usize,
        max: usize,
    },
    #[error("config_files bodies total {bytes} bytes, past the limit of {max}")]
    BodiesTooLarge { bytes: usize, max: usize },
    #[error("config_files[{destination}] collides with the file {other}")]
    Collision { destination: String, other: String },
    #[error(
        "config_files[{destination}] cannot set general_settings.litellm_license or environment_variables.LITELLM_LICENSE"
    )]
    License { destination: String },
}

pub(crate) fn validate(config_files: &BTreeMap<String, String>) -> Result<(), ConfigFixtureError> {
    if config_files.len() > MAX_FILES {
        return Err(ConfigFixtureError::TooManyFiles {
            count: config_files.len(),
            max: MAX_FILES,
        });
    }
    let mut total = 0_usize;
    for (destination, body) in config_files {
        check_destination(destination, body)?;
        total = total.saturating_add(body.len());
    }
    if total > MAX_TOTAL_BYTES {
        return Err(ConfigFixtureError::BodiesTooLarge {
            bytes: total,
            max: MAX_TOTAL_BYTES,
        });
    }
    for destination in config_files.keys() {
        let ancestor = config_files
            .keys()
            .find(|other| destination.starts_with(&format!("{other}/")));
        if let Some(other) = ancestor {
            return Err(ConfigFixtureError::Collision {
                destination: destination.clone(),
                other: other.clone(),
            });
        }
    }
    Ok(())
}

fn check_destination(destination: &str, body: &str) -> Result<(), ConfigFixtureError> {
    if destination.len() > MAX_DESTINATION_BYTES {
        return Err(ConfigFixtureError::DestinationTooLong {
            destination: destination.to_owned(),
            bytes: destination.len(),
            max: MAX_DESTINATION_BYTES,
        });
    }
    if let Some(reason) = unsafe_reason(destination) {
        return Err(ConfigFixtureError::UnsafeDestination {
            destination: destination.to_owned(),
            reason,
        });
    }
    if body.len() > MAX_BODY_BYTES {
        return Err(ConfigFixtureError::BodyTooLarge {
            destination: destination.to_owned(),
            bytes: body.len(),
            max: MAX_BODY_BYTES,
        });
    }
    Ok(())
}

fn unsafe_reason(destination: &str) -> Option<&'static str> {
    if destination.is_empty() {
        return Some("it is empty");
    }
    if destination.contains('\\') {
        return Some("it contains a backslash");
    }
    if destination.contains('\0') {
        return Some("it contains a NUL byte");
    }
    if matches!(destination.as_bytes(), [letter, b':', ..] if letter.is_ascii_alphabetic()) {
        return Some("it starts with a drive prefix");
    }
    let components = destination.split('/').collect::<Vec<_>>();
    if components.len() > MAX_COMPONENTS {
        return Some("it has too many path components");
    }
    for component in components {
        match Path::new(component).components().next() {
            Some(Component::Normal(_)) => {}
            Some(Component::CurDir) => return Some("it contains a dot component"),
            Some(Component::ParentDir) => return Some("it contains a dot-dot component"),
            Some(Component::RootDir | Component::Prefix(_)) => return Some("it is absolute"),
            None => return Some("it contains an empty component"),
        }
    }
    let extension = Path::new(destination)
        .extension()
        .and_then(|value| value.to_str());
    if !matches!(extension, Some("yaml" | "yml")) {
        return Some("it does not end with .yaml or .yml");
    }
    if destination == "config.yaml" {
        return Some("it names the generated root config.yaml");
    }
    None
}

pub(crate) fn write_tree(
    directory: &Path,
    config_files: &BTreeMap<String, String>,
    upstream: &str,
) -> Result<(), OracleError> {
    for (destination, body) in config_files {
        let path = directory.join(destination);
        let parent = path.parent().unwrap_or(directory);
        fs::create_dir_all(parent).map_err(|source| OracleError::Filesystem {
            path: parent.to_owned(),
            source,
        })?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|source| OracleError::Filesystem {
                path: path.clone(),
                source,
            })?;
        let text = body.replace("{{upstream}}", upstream);
        file.write_all(text.as_bytes())
            .map_err(|source| OracleError::Filesystem {
                path: path.clone(),
                source,
            })?;
    }
    Ok(())
}

mod tests;
