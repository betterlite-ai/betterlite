use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::str;

use crate::OracleError;
use crate::websocket_oracle::{
    OracleLimits,
    decode::{FilesystemFixtures, decode_ws_case},
};
use betterlite_postgres_fixture::seed::DatabaseFixture;
use serde::Deserialize;
use serde_json::Value;

use crate::multipart_oracle::error::MultipartError;
use crate::multipart_oracle::request::request_body;
use serde::de::IgnoredAny;

pub mod browser_flow;
mod config_inspection;
mod group;
pub mod helper;

pub(crate) use group::{LoadedCase, ScenarioCase};
pub(crate) use helper::HelperCase;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub(crate) name: String,
    pub(crate) config: String,
    pub(crate) env: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) native_feature_eligibility: bool,
    #[serde(default)]
    pub(crate) request: Box<Request>,
    pub(crate) upstream: Vec<UpstreamAnswer>,
    #[serde(default)]
    pub(crate) compare: Box<Compare>,
    #[serde(default)]
    pub(crate) steps: Option<Vec<browser_flow::Step>>,
    #[serde(default)]
    #[serde(alias = "database_fixture")]
    pub(crate) database: Option<DatabaseFixture>,
}

pub type HttpCase = Case;

impl Case {
    fn common(&self) -> group::CommonFields<'_> {
        group::CommonFields {
            name: &self.name,
            config: &self.config,
            env: &self.env,
            native_feature_eligibility: self.native_feature_eligibility,
            database: self.database.as_ref(),
            upstream: &self.upstream,
        }
    }
}

#[derive(Deserialize)]
struct CaseKind {
    kind: Option<String>,
    scenario: Option<IgnoredAny>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
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

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Compare {
    pub(crate) headers: Vec<String>,
    pub(crate) headers_present: Vec<String>,
    pub(crate) upstream_headers: Vec<String>,
    pub(crate) ignore: Vec<Ignore>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
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
    if probe.kind.as_deref() == Some("websocket") {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/websocket-oracle");
        let fixtures = FilesystemFixtures { root };
        let case = decode_ws_case(text.as_bytes(), &OracleLimits::default(), &fixtures).map_err(
            |source| OracleError::WebSocketCase {
                path: path.to_owned(),
                source,
            },
        )?;
        return Ok(LoadedCase::WebSocket(Box::new(case)));
    }
    if probe.kind.is_some() {
        let case: HelperCase = serde_saphyr::from_str(text).map_err(parse_error)?;
        helper::validate_helper(path, &case)?;
        return Ok(LoadedCase::Helper(case));
    }
    if probe.scenario.is_some() {
        let case: ScenarioCase = serde_saphyr::from_str(text).map_err(parse_error)?;
        group::validate_scenario_case(path, &case)?;
        return Ok(LoadedCase::Scenario(case));
    }
    let case = decode_case(path, text.as_bytes())?;
    Ok(LoadedCase::Http(case))
}

pub fn decode_http_body(bytes: &[u8]) -> Result<RequestBody, MultipartError> {
    let text = str::from_utf8(bytes).map_err(MultipartError::RequestUtf8)?;
    let request: Request = serde_saphyr::from_str(text)
        .map_err(|source| MultipartError::RequestSpecification(Box::new(source)))?;
    request_body(&request.body, request.body_kind)
}

pub fn decode_case(path: &Path, bytes: &[u8]) -> Result<HttpCase, OracleError> {
    let text = str::from_utf8(bytes).map_err(|source| OracleError::CaseUtf8 { source })?;
    let case: Case = serde_saphyr::from_str(text).map_err(|source| OracleError::CaseParse {
        path: path.to_owned(),
        source: Box::new(source),
    })?;
    validate(path, &case)?;
    Ok(case)
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

pub(crate) fn validate(path: &Path, case: &Case) -> Result<(), OracleError> {
    group::validate_common(path, &case.common())?;
    validate_request_or_steps(path, case)
}

// Serde defaults the unused side of request/compare or steps, so emptiness stands in for absence.
fn validate_request_or_steps(path: &Path, case: &Case) -> Result<(), OracleError> {
    let invalid = |message: &str| OracleError::CaseInvalid {
        path: path.to_owned(),
        message: message.to_owned(),
    };
    let plain_request_unset =
        *case.request == Request::default() && *case.compare == Compare::default();
    if let Some(steps) = &case.steps {
        if !plain_request_unset {
            return Err(invalid("a case cannot set both request/compare and steps"));
        }
        return browser_flow::validate_steps(path, steps);
    }
    if case.request.path.is_empty() {
        return Err(invalid("request.path cannot be empty"));
    }
    request_body(&case.request.body, case.request.body_kind).map_err(OracleError::Multipart)?;
    validate_ignores(path, &case.compare.ignore)
}

pub(crate) fn validate_ignores(path: &Path, ignores: &[Ignore]) -> Result<(), OracleError> {
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
