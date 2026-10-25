use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::str;

use crate::OracleError;
use crate::helpers::HelperCall;
use betterlite_postgres_fixture::seed::DatabaseFixture;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub(crate) name: String,
    pub(crate) config: String,
    pub(crate) env: BTreeMap<String, String>,
    pub(crate) request: Box<Request>,
    pub(crate) upstream: Vec<UpstreamAnswer>,
    pub(crate) compare: Box<Compare>,
    #[serde(default)]
    #[serde(alias = "database_fixture")]
    pub(crate) database: Option<DatabaseFixture>,
}

pub type HttpCase = Case;

#[derive(Debug, Clone)]
pub(crate) enum LoadedCase {
    Http(Case),
    Helper(HelperCase),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HelperCase {
    pub(crate) kind: HelperKind,
    pub(crate) name: String,
    pub(crate) call: HelperCall,
    pub(crate) env: BTreeMap<String, String>,
    pub(crate) compare: HelperCompare,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HelperKind {
    Helper,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HelperCompare {
    pub(crate) ignore: Vec<Ignore>,
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

pub(crate) fn load_cases(paths: &[PathBuf]) -> Result<Vec<LoadedCase>, OracleError> {
    let mut files = Vec::new();
    for path in paths {
        collect_yaml(path, &mut files)?;
    }
    files.sort();
    let mut cases = Vec::with_capacity(files.len());
    for path in files {
        let text = fs::read_to_string(&path).map_err(|source| OracleError::CaseRead {
            path: path.clone(),
            source,
        })?;
        cases.push(parse_case(&path, &text)?);
    }
    if cases.is_empty() {
        return Err(OracleError::CaseInvalid {
            path: PathBuf::from("<arguments>"),
            message: "no YAML cases found".to_owned(),
        });
    }
    Ok(cases)
}

pub(crate) fn parse_case(path: &Path, text: &str) -> Result<LoadedCase, OracleError> {
    if path
        .components()
        .any(|component| component.as_os_str() == "helpers")
    {
        let case: HelperCase =
            serde_saphyr::from_str(text).map_err(|source| OracleError::CaseParse {
                path: path.to_owned(),
                source: Box::new(source),
            })?;
        validate_helper(path, &case)?;
        return Ok(LoadedCase::Helper(case));
    }
    let case = decode_http_case_at(text.as_bytes(), path)?;
    validate(path, &case)?;
    Ok(LoadedCase::Http(case))
}

pub fn decode_http_case(bytes: &[u8]) -> Result<HttpCase, OracleError> {
    let path = Path::new("<bytes>");
    if bytes.is_empty() {
        return decode_http_case_at(bytes, path);
    }
    decode_http_case_at(bytes, path)
}

fn decode_http_case_at(bytes: &[u8], path: &Path) -> Result<HttpCase, OracleError> {
    let text = str::from_utf8(bytes).map_err(|source| OracleError::CaseUtf8 { source })?;
    let case: Case = serde_saphyr::from_str(text).map_err(|source| {
        if path == Path::new("<bytes>") {
            OracleError::HttpCaseParse {
                source: Box::new(source),
            }
        } else {
            OracleError::CaseParse {
                path: path.to_owned(),
                source: Box::new(source),
            }
        }
    })?;
    validate(path, &case)?;
    Ok(case)
}

pub fn parse_helper_case(source: &str) -> Result<(), String> {
    let case: HelperCase = serde_saphyr::from_str(source).map_err(|error| error.to_string())?;
    validate_helper(Path::new("<helper>"), &case).map_err(|error| error.to_string())
}

fn collect_yaml(path: &Path, files: &mut Vec<PathBuf>) -> Result<(), OracleError> {
    let read_error = |source| OracleError::CaseRead {
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

fn validate_helper(path: &Path, case: &HelperCase) -> Result<(), OracleError> {
    let invalid = |message: &str| OracleError::CaseInvalid {
        path: path.to_owned(),
        message: message.to_owned(),
    };
    if !matches!(&case.kind, HelperKind::Helper) {
        return Err(invalid("helper cases must have kind: helper"));
    }
    if case.name.is_empty() {
        return Err(invalid("name cannot be empty"));
    }
    validate_ignores(path, &case.compare.ignore)
}

pub(crate) fn validate(path: &Path, case: &Case) -> Result<(), OracleError> {
    let invalid = |message: &str| OracleError::CaseInvalid {
        path: path.to_owned(),
        message: message.to_owned(),
    };
    if case.name.is_empty() || case.config.is_empty() || case.request.path.is_empty() {
        return Err(invalid("name, config, and request.path cannot be empty"));
    }
    if let Some(database) = &case.database {
        database
            .seed
            .validate()
            .map_err(|source| OracleError::HttpCaseFixture { source })?;
    }
    for answer in &case.upstream {
        if answer.body.is_some() == answer.events.is_some() {
            return Err(invalid(
                "each upstream answer must contain exactly one of body or events",
            ));
        }
    }
    validate_ignores(path, &case.compare.ignore)
}

fn validate_ignores(path: &Path, ignores: &[Ignore]) -> Result<(), OracleError> {
    let invalid = |message: &str| OracleError::CaseInvalid {
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
