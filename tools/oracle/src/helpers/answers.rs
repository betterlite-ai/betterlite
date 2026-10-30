use std::collections::BTreeMap;
use std::env::{self, VarError};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};
use ulid::Ulid;

use super::python::python_stdout;
use super::{HelperCall, HelperOutput};
use crate::OracleError;
use crate::process::sync_litellm;

const KEYED_FILES: [&str; 4] = [
    "tools/oracle/litellm/uv.lock",
    "tools/oracle/litellm/pyproject.toml",
    "tools/oracle/litellm/.python-version",
    "tools/oracle/helpers/dispatch.py",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Layer {
    Own,
    Shared,
}

pub(super) struct Answers {
    pub(super) shared: PathBuf,
    pub(super) own: PathBuf,
    pub(super) write: Layer,
}

#[derive(serde::Serialize)]
struct Key<'call> {
    files: BTreeMap<String, Option<String>>,
    call: &'call HelperCall,
    env: &'call BTreeMap<String, String>,
}

impl Answers {
    pub(super) fn from_env(root: &Path) -> Result<Self, OracleError> {
        let write = match env::var("BETTERLITE_ORACLE_CACHE").as_deref() {
            Err(VarError::NotPresent) | Ok("own") => Layer::Own,
            Ok("shared") => Layer::Shared,
            Ok(_) | Err(VarError::NotUnicode(_)) => return Err(OracleError::CacheMode),
        };
        let home = env::var_os("HOME").ok_or(OracleError::CacheHome)?;
        Ok(Self {
            shared: PathBuf::from(home).join(".cache/betterlite-oracle"),
            own: root.join("target/oracle-answers"),
            write,
        })
    }

    pub(super) fn answer(
        &self,
        root: &Path,
        call: &HelperCall,
        case_env: &BTreeMap<String, String>,
    ) -> Result<HelperOutput, OracleError> {
        let name = format!("{}.json", key(root, call, case_env)?);
        for folder in [&self.shared, &self.own] {
            let path = folder.join(&name);
            match fs::read(&path) {
                Ok(stored) => {
                    return serde_json::from_slice(&stored).map_err(OracleError::HelperJson);
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(source) => return Err(OracleError::Filesystem { path, source }),
            }
        }
        sync_litellm(root)?;
        let stdout_output = python_stdout(root, call, case_env)?;
        let output = serde_json::from_slice(&stdout_output).map_err(OracleError::HelperJson)?;
        let folder = match self.write {
            Layer::Own => &self.own,
            Layer::Shared => &self.shared,
        };
        fs::create_dir_all(folder).map_err(|source| OracleError::Filesystem {
            path: folder.clone(),
            source,
        })?;
        let partial = folder.join(format!("{name}.{}", Ulid::generate()));
        fs::write(&partial, &stdout_output).map_err(|source| OracleError::Filesystem {
            path: partial.clone(),
            source,
        })?;
        fs::rename(&partial, folder.join(&name)).map_err(|source| OracleError::Filesystem {
            path: partial,
            source,
        })?;
        Ok(output)
    }
}

pub(super) fn key(
    root: &Path,
    call: &HelperCall,
    case_env: &BTreeMap<String, String>,
) -> Result<String, OracleError> {
    let module = format!("tools/oracle/helpers/{}.py", call.operation);
    let mut files = BTreeMap::new();
    for name in KEYED_FILES.into_iter().chain([module.as_str()]) {
        let path = root.join(name);
        let digest = match fs::read(&path) {
            Ok(bytes) => Some(hex::encode(Sha256::digest(bytes))),
            Err(error) if error.kind() == io::ErrorKind::NotFound && name == module => None,
            Err(source) => return Err(OracleError::Filesystem { path, source }),
        };
        files.insert(name.to_owned(), digest);
    }
    let encoded = serde_json::to_vec(&Key {
        files,
        call,
        env: case_env,
    })
    .map_err(OracleError::Json)?;
    Ok(hex::encode(Sha256::digest(encoded)))
}
