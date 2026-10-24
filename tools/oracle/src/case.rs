use std::collections::BTreeMap;
use std::fs;
use std::io::Error;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Case {
    pub(crate) name: String,
    pub(crate) config: String,
    pub(crate) env: BTreeMap<String, String>,
    pub(crate) request: Request,
    pub(crate) upstream: Vec<UpstreamAnswer>,
    pub(crate) compare: Compare,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpstreamAnswer {
    pub(crate) status: u16,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Option<Value>,
    pub(crate) events: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Compare {
    pub(crate) headers: Vec<String>,
    pub(crate) headers_present: Vec<String>,
    pub(crate) upstream_headers: Vec<String>,
    pub(crate) ignore: Vec<Ignore>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Ignore {
    pub(crate) pointer: String,
    pub(crate) why: String,
}

#[derive(Debug, Error)]
pub(crate) enum CaseError {
    #[error("cannot read case {path}: {source}")]
    Read { path: PathBuf, source: Error },
    #[error("cannot parse case {path}: {source}")]
    Parse {
        path: PathBuf,
        source: serde_yaml::Error,
    },
    #[error("invalid case {path}: {message}")]
    Invalid { path: PathBuf, message: String },
}

pub(crate) fn load_cases(paths: &[PathBuf]) -> Result<Vec<Case>, CaseError> {
    let mut files = Vec::new();
    for path in paths {
        collect_yaml(path, &mut files)?;
    }
    files.sort();
    let mut cases = Vec::with_capacity(files.len());
    for path in files {
        let text = fs::read_to_string(&path).map_err(|source| CaseError::Read {
            path: path.clone(),
            source,
        })?;
        let case: Case = serde_yaml::from_str(&text).map_err(|source| CaseError::Parse {
            path: path.clone(),
            source,
        })?;
        validate(&path, &case)?;
        cases.push(case);
    }
    if cases.is_empty() {
        return Err(CaseError::Invalid {
            path: PathBuf::from("<arguments>"),
            message: "no YAML cases found".to_owned(),
        });
    }
    Ok(cases)
}

fn collect_yaml(path: &Path, files: &mut Vec<PathBuf>) -> Result<(), CaseError> {
    let read_error = |source| CaseError::Read {
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

pub(crate) fn validate(path: &Path, case: &Case) -> Result<(), CaseError> {
    let invalid = |message: &str| CaseError::Invalid {
        path: path.to_owned(),
        message: message.to_owned(),
    };
    if case.name.is_empty() || case.config.is_empty() || case.request.path.is_empty() {
        return Err(invalid("name, config, and request.path cannot be empty"));
    }
    for answer in &case.upstream {
        if answer.body.is_some() == answer.events.is_some() {
            return Err(invalid(
                "each upstream answer must contain exactly one of body or events",
            ));
        }
    }
    for ignored in &case.compare.ignore {
        if !ignored.pointer.is_empty() && !ignored.pointer.starts_with('/') {
            return Err(invalid("compare.ignore pointers must be JSON pointers"));
        }
        if ignored.why.is_empty() {
            return Err(invalid("compare.ignore entries need a why"));
        }
    }
    Ok(())
}
