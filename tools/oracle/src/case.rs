use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::str;

use crate::OracleError;
use betterlite_postgres_fixture::seed::DatabaseFixture;
use serde::Deserialize;
use serde_json::Value;

use crate::multipart_oracle::error::MultipartError;
use crate::multipart_oracle::request::{answer_body, request_body};
use crate::routing_scenarios;
use serde::de::IgnoredAny;

pub mod browser_flow;
mod config_inspection;
mod helper;

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

// A scenario case carries its own fields so plain Case literals never gain an unused step field.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScenarioCase {
    pub(crate) name: String,
    pub(crate) config: String,
    pub(crate) env: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) native_feature_eligibility: bool,
    pub(crate) upstream: Vec<UpstreamAnswer>,
    pub(crate) scenario: routing_scenarios::Scenario,
    #[serde(default)]
    #[serde(alias = "database_fixture")]
    pub(crate) database: Option<DatabaseFixture>,
}

impl ScenarioCase {
    // Bridges to Case only where out-of-scope code (artifacts::prepare/write) requires the concrete type.
    pub(crate) fn as_case(&self) -> Case {
        Case {
            name: self.name.clone(),
            config: self.config.clone(),
            env: self.env.clone(),
            native_feature_eligibility: self.native_feature_eligibility,
            request: Box::new(Request::default()),
            upstream: self.upstream.clone(),
            compare: Box::new(Compare::default()),
            steps: None,
            database: self.database.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum LoadedCase {
    Http(Case),
    Scenario(ScenarioCase),
    Helper(HelperCase),
}

// Both Http and Scenario cases run through the same gateway group, so comparisons share one type.
#[derive(Debug, Clone)]
pub(crate) enum GroupCase {
    Http(Case),
    Scenario(ScenarioCase),
}

impl GroupCase {
    pub(crate) fn name(&self) -> &str {
        match self {
            Self::Http(case) => &case.name,
            Self::Scenario(case) => &case.name,
        }
    }

    pub(crate) fn config(&self) -> &str {
        match self {
            Self::Http(case) => &case.config,
            Self::Scenario(case) => &case.config,
        }
    }

    pub(crate) const fn env(&self) -> &BTreeMap<String, String> {
        match self {
            Self::Http(case) => &case.env,
            Self::Scenario(case) => &case.env,
        }
    }

    pub(crate) const fn native_feature_eligibility(&self) -> bool {
        match self {
            Self::Http(case) => case.native_feature_eligibility,
            Self::Scenario(case) => case.native_feature_eligibility,
        }
    }

    pub(crate) const fn database(&self) -> Option<&DatabaseFixture> {
        match self {
            Self::Http(case) => case.database.as_ref(),
            Self::Scenario(case) => case.database.as_ref(),
        }
    }

    pub(crate) fn upstream(&self) -> &[UpstreamAnswer] {
        match self {
            Self::Http(case) => &case.upstream,
            Self::Scenario(case) => &case.upstream,
        }
    }

    // Lets a group untouched by a scenario run through the plain Case pipeline instead.
    pub(crate) fn all_http(cases: &[Self]) -> Option<Vec<Case>> {
        cases
            .iter()
            .map(|case| match case {
                Self::Http(http_case) => Some(http_case.clone()),
                Self::Scenario(_) => None,
            })
            .collect()
    }

    // Satisfies artifacts::prepare's concrete Case parameter; a Scenario case bridges through ScenarioCase::as_case.
    pub(crate) fn as_case(&self) -> Case {
        match self {
            Self::Http(case) => case.clone(),
            Self::Scenario(case) => case.as_case(),
        }
    }
}

#[derive(Deserialize)]
struct CaseKind {
    kind: Option<helper::HelperKind>,
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
    if probe.kind.is_some() {
        let case: HelperCase = serde_saphyr::from_str(text).map_err(parse_error)?;
        helper::validate_helper(path, &case)?;
        return Ok(LoadedCase::Helper(case));
    }
    if probe.scenario.is_some() {
        let case: ScenarioCase = serde_saphyr::from_str(text).map_err(parse_error)?;
        validate_scenario_case(path, &case)?;
        return Ok(LoadedCase::Scenario(case));
    }
    let case = decode_case(path, text.as_bytes())?;
    Ok(LoadedCase::Http(case))
}

pub fn parse_helper_case(source: &str) -> Result<(), String> {
    let case: HelperCase = serde_saphyr::from_str(source).map_err(|error| error.to_string())?;
    helper::validate_helper(Path::new("<helper>"), &case).map_err(|error| error.to_string())
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

pub fn decode_html_form_capture(
    bytes: &[u8],
) -> Result<browser_flow::form::FormProjection, browser_flow::form::FormCaptureError> {
    let text = str::from_utf8(bytes).map_err(browser_flow::form::FormCaptureError::Utf8)?;
    browser_flow::form::parse_form_projection(text)
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
    validate_common(
        path,
        &CommonFields {
            name: &case.name,
            config: &case.config,
            env: &case.env,
            native_feature_eligibility: case.native_feature_eligibility,
            database: case.database.as_ref(),
            upstream: &case.upstream,
        },
    )?;
    validate_request_or_steps(path, case)
}

fn validate_scenario_case(path: &Path, case: &ScenarioCase) -> Result<(), OracleError> {
    validate_common(
        path,
        &CommonFields {
            name: &case.name,
            config: &case.config,
            env: &case.env,
            native_feature_eligibility: case.native_feature_eligibility,
            database: case.database.as_ref(),
            upstream: &case.upstream,
        },
    )?;
    routing_scenarios::validate_in_case(path, &case.scenario)
}

struct CommonFields<'input> {
    name: &'input str,
    config: &'input str,
    env: &'input BTreeMap<String, String>,
    native_feature_eligibility: bool,
    database: Option<&'input DatabaseFixture>,
    upstream: &'input [UpstreamAnswer],
}

fn validate_common(path: &Path, fields: &CommonFields<'_>) -> Result<(), OracleError> {
    let invalid = |message: &str| OracleError::CaseInvalid {
        path: path.to_owned(),
        message: message.to_owned(),
    };
    if fields.name.is_empty() || fields.config.is_empty() {
        return Err(invalid("name and config cannot be empty"));
    }
    if fields.env.contains_key("LITELLM_LICENSE") {
        return Err(invalid("env cannot set LITELLM_LICENSE"));
    }
    if fields.env.contains_key("LITELLM_WORKER_STARTUP_HOOKS") {
        return Err(invalid("env cannot set LITELLM_WORKER_STARTUP_HOOKS"));
    }
    if fields.native_feature_eligibility && fields.env.contains_key("PYTHONPATH") {
        return Err(invalid("env cannot set PYTHONPATH on an eligible case"));
    }
    if config_inspection::requests_license(path, fields.config)? {
        return Err(invalid(
            "config cannot set general_settings.litellm_license or environment_variables.LITELLM_LICENSE",
        ));
    }
    if let Some(fixture) = fields.database {
        fixture
            .seed
            .validate()
            .map_err(|source| OracleError::HttpCaseFixture { source })?;
    }
    for answer in fields.upstream {
        if answer.body.is_some() == answer.events.is_some() {
            return Err(invalid(
                "each upstream answer must contain exactly one of body or events",
            ));
        }
        answer_body(answer).map_err(OracleError::Multipart)?;
    }
    Ok(())
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
