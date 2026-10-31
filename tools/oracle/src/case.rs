use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::str;

use crate::OracleError;
use crate::helpers::HelperCall;
use betterlite_postgres_fixture::seed::DatabaseFixture;
use serde::Deserialize;
use serde_json::Value;

use crate::multipart_oracle::error::MultipartError;
use crate::multipart_oracle::request::{answer_body, request_body};

mod config_inspection;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub(crate) name: String,
    pub(crate) config: String,
    pub(crate) env: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) native_feature_eligibility: bool,
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

#[derive(Deserialize)]
struct CaseKind {
    kind: Option<HelperKind>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HelperCompare {
    pub(crate) ignore: Vec<Ignore>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Value,
    #[serde(default)]
    pub(crate) body_kind: BodyKind,
}

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
    let parse_error = |source: serde_saphyr::Error| OracleError::CaseParse {
        path: path.to_owned(),
        source: Box::new(source),
    };
    // the case says what it is, so its folder never decides
    let probe: CaseKind = serde_saphyr::from_str(text).map_err(parse_error)?;
    if probe.kind.is_some() {
        let case: HelperCase = serde_saphyr::from_str(text).map_err(parse_error)?;
        validate_helper(path, &case)?;
        return Ok(LoadedCase::Helper(case));
    }
    let case = decode_case(path, text.as_bytes())?;
    Ok(LoadedCase::Http(case))
}

pub fn decode_http_case(bytes: &[u8]) -> Result<HttpCase, OracleError> {
    let path = Path::new("<bytes>");
    decode_case(path, bytes)
}

pub fn decode_http_body(bytes: &[u8]) -> Result<RequestBody, MultipartError> {
    let text = str::from_utf8(bytes).map_err(MultipartError::RequestUtf8)?;
    let request: Request = serde_saphyr::from_str(text)
        .map_err(|source| MultipartError::RequestSpecification(Box::new(source)))?;
    request_body(&request.body, request.body_kind)
}

fn decode_case(path: &Path, bytes: &[u8]) -> Result<HttpCase, OracleError> {
    let text = str::from_utf8(bytes).map_err(|source| OracleError::CaseUtf8 { source })?;
    let case: Case = serde_saphyr::from_str(text).map_err(|source| OracleError::CaseParse {
        path: path.to_owned(),
        source: Box::new(source),
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
    if case.env.contains_key("LITELLM_LICENSE") {
        return Err(invalid("env cannot set LITELLM_LICENSE"));
    }
    if case.env.contains_key("LITELLM_WORKER_STARTUP_HOOKS") {
        return Err(invalid("env cannot set LITELLM_WORKER_STARTUP_HOOKS"));
    }
    if case.native_feature_eligibility && case.env.contains_key("PYTHONPATH") {
        return Err(invalid("env cannot set PYTHONPATH on an eligible case"));
    }
    if config_inspection::requests_license(path, &case.config)? {
        return Err(invalid(
            "config cannot set general_settings.litellm_license or environment_variables.LITELLM_LICENSE",
        ));
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
        answer_body(answer).map_err(OracleError::Multipart)?;
    }
    // the request body decodes here once so a bad specification stops the run before any gateway starts
    request_body(&case.request.body, case.request.body_kind).map_err(OracleError::Multipart)?;
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
