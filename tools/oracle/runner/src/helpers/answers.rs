use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
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

const HELPERS: &str = "tools/oracle/helpers";

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
    let mut files = BTreeMap::new();
    for name in KEYED_FILES {
        let path = root.join(name);
        let bytes =
            fs::read(&path).map_err(|source| HelperRunError::Filesystem { path, source })?;
        files.insert(name.to_owned(), Some(hex::encode(Sha256::digest(bytes))));
    }
    helper_sources(root, &call.operation, &mut files)?;
    let encoded = serde_json::to_vec(&Key {
        files,
        call: BTreeMap::from([(call.operation.as_str(), &call.input)]),
        env: case_env,
    })
    .map_err(HelperRunError::Json)?;
    Ok(hex::encode(Sha256::digest(encoded)))
}

// Python finds sibling helpers by their top-level name, so each helper an import line names is keyed too
fn helper_sources(
    root: &Path,
    operation: &str,
    files: &mut BTreeMap<String, Option<String>>,
) -> Result<(), HelperRunError> {
    let mut pending = vec![operation.to_owned()];
    let mut seen = BTreeSet::new();
    while let Some(name) = pending.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        let module = format!("{HELPERS}/{name}.py");
        let sources = if root.join(&module).is_file() {
            vec![module]
        } else if root.join(HELPERS).join(&name).is_dir() {
            package_files(root, &name)?
        } else {
            if name == operation {
                files.insert(module, None);
            }
            continue;
        };
        for relative in sources {
            let path = root.join(&relative);
            let bytes =
                fs::read(&path).map_err(|source| HelperRunError::Filesystem { path, source })?;
            if Path::new(&relative).extension() == Some(OsStr::new("py")) {
                pending.extend(imported_names(str::from_utf8(&bytes).unwrap_or_default()));
            }
            files.insert(relative, Some(hex::encode(Sha256::digest(bytes))));
        }
    }
    Ok(())
}

fn imported_names(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    for text_line in text.lines() {
        let trimmed = text_line.trim_start();
        let Some(rest) = trimmed
            .strip_prefix("import ")
            .or_else(|| trimmed.strip_prefix("from "))
        else {
            continue;
        };
        let imported = rest.split([' ', '.', ',']).next().unwrap_or_default();
        let identifier = imported
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_');
        if !imported.is_empty() && identifier {
            names.push(imported.to_owned());
        }
    }
    names
}

// bytecode appears after the first run, so __pycache__ stays out of the key
fn package_files(root: &Path, name: &str) -> Result<Vec<String>, HelperRunError> {
    let mut found = Vec::new();
    let mut folders = vec![format!("{HELPERS}/{name}")];
    while let Some(folder) = folders.pop() {
        let path = root.join(&folder);
        let entries = fs::read_dir(&path).map_err(|source| HelperRunError::Filesystem {
            path: path.clone(),
            source,
        })?;
        for listed in entries {
            let entry = listed.map_err(|source| HelperRunError::Filesystem {
                path: path.clone(),
                source,
            })?;
            let file_name = entry
                .file_name()
                .into_string()
                .map_err(|_unreadable| HelperRunError::HelperFileName { path: entry.path() })?;
            if file_name == "__pycache__" {
                continue;
            }
            let file_type = entry
                .file_type()
                .map_err(|source| HelperRunError::Filesystem {
                    path: entry.path(),
                    source,
                })?;
            let relative = format!("{folder}/{file_name}");
            if file_type.is_dir() {
                folders.push(relative);
            } else {
                found.push(relative);
            }
        }
    }
    Ok(found)
}
