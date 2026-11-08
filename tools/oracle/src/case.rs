use std::collections::BTreeMap;
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
use crate::oracle_comparison_contracts::{self, Contracts, ExpectedDifference, NumericRule};
use serde::de::IgnoredAny;

pub mod browser_flow;
mod config_inspection;
mod group;
pub mod helper;
mod load;

pub(crate) use group::{LoadedCase, ScenarioCase};
pub(crate) use helper::HelperCase;
pub(crate) use load::{load_cases, load_probe_cases, load_sdk_http_cases};

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
    pub(crate) fn with_self_check(&self, self_check: bool) -> Self {
        let mut owned = self.clone();
        mark_compare_self_check(&mut owned.compare, self_check);
        if let Some(steps) = &mut owned.steps {
            for step in steps {
                mark_compare_self_check(&mut step.compare, self_check);
            }
        }
        owned
    }

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

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(from = "CompareWire")]
pub(crate) struct Compare {
    pub(crate) headers: Vec<String>,
    pub(crate) headers_present: Vec<String>,
    pub(crate) upstream_headers: Vec<String>,
    pub(crate) ignore: Vec<Ignore>,
    // one pointer keeps a scenario step under the enum variant size lint
    pub(crate) contracts: Option<Box<Contracts>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompareWire {
    headers: Vec<String>,
    headers_present: Vec<String>,
    upstream_headers: Vec<String>,
    ignore: Vec<Ignore>,
    #[serde(default)]
    numeric: Vec<NumericRule>,
    #[serde(default)]
    expected_difference: Vec<ExpectedDifference>,
}

impl From<CompareWire> for Compare {
    fn from(wire: CompareWire) -> Self {
        let declared = !wire.numeric.is_empty() || !wire.expected_difference.is_empty();
        let contracts = declared.then(|| {
            Box::new(Contracts {
                numeric: wire.numeric,
                expected_difference: wire.expected_difference,
                self_check: false,
            })
        });
        Self {
            headers: wire.headers,
            headers_present: wire.headers_present,
            upstream_headers: wire.upstream_headers,
            ignore: wire.ignore,
            contracts,
        }
    }
}

pub(crate) fn mark_compare_self_check(compare: &mut Compare, self_check: bool) {
    if let Some(contracts) = compare.contracts.as_mut() {
        contracts.self_check = self_check;
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Ignore {
    pub(crate) pointer: String,
    pub(crate) why: String,
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
    let case: Case = load::parse_case_text(path, text)?;
    validate(path, &case)?;
    Ok(case)
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
    validate_ignores(path, &case.compare.ignore)?;
    validate_contracts(path, &case.compare)
}

pub(crate) fn validate_contracts(path: &Path, compare: &Compare) -> Result<(), OracleError> {
    let Some(contracts) = compare.contracts.as_deref() else {
        return Ok(());
    };
    let outcome =
        oracle_comparison_contracts::validate(&contracts.numeric, &contracts.expected_difference);
    outcome.map_err(|error| OracleError::CaseInvalid {
        path: path.to_owned(),
        message: error.to_string(),
    })
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
