use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use crate::error::HelperRunError;
use helper::HelperCase;

pub mod helper;

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BodyKind {
    #[default]
    Json,
    Multipart,
    Bytes,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum RequestBody {
    #[default]
    Empty,
    Json(Vec<u8>),
    Raw {
        content_type: String,
        bytes: Vec<u8>,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpstreamAnswer {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Option<Value>,
    pub events: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Ignore {
    pub pointer: String,
    pub why: String,
}

pub fn validate_ignores(path: &Path, ignores: &[Ignore]) -> Result<(), HelperRunError> {
    let invalid = |message: &str| HelperRunError::CaseInvalid {
        path: path.to_owned(),
        message: message.to_owned(),
    };
    for ignored in ignores {
        if !ignored.pointer.is_empty() && !ignored.pointer.starts_with('/') {
            return Err(invalid("compare.ignore pointers must be JSON pointers"));
        }
        if ignored.why.is_empty() {
            return Err(invalid("compare.ignore entries need a why"));
        }
    }
    Ok(())
}

pub fn load_helper_cases(paths: &[PathBuf]) -> Result<Vec<HelperCase>, HelperRunError> {
    read_case_files(paths)?
        .iter()
        .map(|(path, text)| helper::parse_helper_case(path, text))
        .collect()
}

pub fn read_case_files(paths: &[PathBuf]) -> Result<Vec<(PathBuf, String)>, HelperRunError> {
    let mut files = Vec::new();
    for path in paths {
        collect_yaml(path, &mut files)?;
    }
    if files.is_empty() {
        return Err(HelperRunError::CaseInvalid {
            path: PathBuf::from("<arguments>"),
            message: "no YAML cases found".to_owned(),
        });
    }
    files.sort();
    let mut texts = Vec::with_capacity(files.len());
    for path in files {
        let text = fs::read_to_string(&path).map_err(|source| HelperRunError::CaseRead {
            path: path.clone(),
            source,
        })?;
        texts.push((path, text));
    }
    Ok(texts)
}

fn collect_yaml(path: &Path, files: &mut Vec<PathBuf>) -> Result<(), HelperRunError> {
    let read_error = |source| HelperRunError::CaseRead {
        path: path.to_owned(),
        source,
    };
    let metadata = fs::metadata(path).map_err(&read_error)?;
    if metadata.is_file() {
        if path
            .extension()
            .is_some_and(|extension| extension == "yaml" || extension == "yml")
        {
            files.push(path.to_owned());
        }
        return Ok(());
    }
    let entries = fs::read_dir(path).map_err(&read_error)?;
    for result in entries {
        let entry = result.map_err(&read_error)?;
        collect_yaml(&entry.path(), files)?;
    }
    Ok(())
}
