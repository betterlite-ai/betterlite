use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest as _, Sha256};
use ulid::Ulid;

use super::python::{process_environment, python_stdout};
use super::sync_litellm;
use super::{HelperCall, HelperOutput};
use crate::error::HelperRunError;

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
    pub(super) process_env: BTreeMap<&'static str, OsString>,
}

#[derive(serde::Serialize)]
struct Key<'call> {
    files: BTreeMap<String, Option<String>>,
    // the one-entry map a case file writes, so cached answers keep their keys
    call: BTreeMap<&'call str, &'call Value>,
    env: &'call BTreeMap<String, String>,
}

impl Answers {
    pub(super) fn from_env(root: &Path) -> Result<Self, HelperRunError> {
        let process_env = process_environment();
        let write = match process_env
            .get("BETTERLITE_ORACLE_CACHE")
            .map(|mode| mode.to_str())
        {
            None | Some(Some("own")) => Layer::Own,
            Some(Some("shared")) => Layer::Shared,
            Some(_) => return Err(HelperRunError::CacheMode),
        };
        let home = process_env.get("HOME").ok_or(HelperRunError::CacheHome)?;
        Ok(Self {
            shared: Path::new(home).join(".cache/betterlite-oracle"),
            own: root.join("target/oracle-answers"),
            write,
            process_env,
        })
    }

    fn stored(&self, name: &str) -> Result<Option<HelperOutput>, HelperRunError> {
        for (layer, folder) in [(Layer::Shared, &self.shared), (Layer::Own, &self.own)] {
            let path = folder.join(name);
            let stored = match fs::read(&path) {
                Ok(stored) => stored,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(source) => return Err(HelperRunError::Filesystem { path, source }),
            };
            if layer == Layer::Shared {
                // CI's oracle answers job keeps a shared answer only while some run reads it, and learns that from these markers
                let used = self.own.join("used");
                fs::create_dir_all(&used).map_err(|source| HelperRunError::Filesystem {
                    path: used.clone(),
                    source,
                })?;
                let marker = used.join(name);
                fs::write(&marker, b"").map_err(|source| HelperRunError::Filesystem {
                    path: marker,
                    source,
                })?;
            }
            return serde_json::from_slice(&stored)
                .map(Some)
                .map_err(HelperRunError::HelperJson);
        }
        Ok(None)
    }

    pub(super) fn answer(
        &self,
        root: &Path,
        call: &HelperCall,
        case_env: &BTreeMap<String, String>,
    ) -> Result<HelperOutput, HelperRunError> {
        let name = format!("{}.json", key(root, call, case_env)?);
        if let Some(stored) = self.stored(&name)? {
            return Ok(stored);
        }
        sync_litellm(root)?;
        let stdout_output = python_stdout(root, call, case_env, &self.process_env)?;
        let output = serde_json::from_slice(&stdout_output).map_err(HelperRunError::HelperJson)?;
        let folder = match self.write {
            Layer::Own => &self.own,
            Layer::Shared => &self.shared,
        };
        fs::create_dir_all(folder).map_err(|source| HelperRunError::Filesystem {
            path: folder.clone(),
            source,
        })?;
        let partial = folder.join(format!("{name}.{}", Ulid::generate()));
        fs::write(&partial, &stdout_output).map_err(|source| HelperRunError::Filesystem {
            path: partial.clone(),
            source,
        })?;
        fs::rename(&partial, folder.join(&name)).map_err(|source| HelperRunError::Filesystem {
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
) -> Result<String, HelperRunError> {
    let module = format!("tools/oracle/helpers/{}.py", call.operation);
    let mut files = BTreeMap::new();
    for name in KEYED_FILES.into_iter().chain([module.as_str()]) {
        let path = root.join(name);
        let digest = match fs::read(&path) {
            Ok(bytes) => Some(hex::encode(Sha256::digest(bytes))),
            Err(error) if error.kind() == io::ErrorKind::NotFound && name == module => None,
            Err(source) => return Err(HelperRunError::Filesystem { path, source }),
        };
        files.insert(name.to_owned(), digest);
    }
    let encoded = serde_json::to_vec(&Key {
        files,
        call: BTreeMap::from([(call.operation.as_str(), &call.input)]),
        env: case_env,
    })
    .map_err(HelperRunError::Json)?;
    Ok(hex::encode(Sha256::digest(encoded)))
}
