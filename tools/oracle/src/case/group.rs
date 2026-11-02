use std::collections::BTreeMap;
use std::path::Path;

use betterlite_postgres_fixture::seed::DatabaseFixture;
use serde::Deserialize;

use super::{Case, Compare, HelperCase, Request, UpstreamAnswer};
use crate::OracleError;
use crate::multipart_oracle::request::answer_body;
use crate::routing_scenarios;

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
    fn common(&self) -> CommonFields<'_> {
        CommonFields {
            name: &self.name,
            config: &self.config,
            env: &self.env,
            native_feature_eligibility: self.native_feature_eligibility,
            database: self.database.as_ref(),
            upstream: &self.upstream,
        }
    }

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

pub(super) fn validate_scenario_case(path: &Path, case: &ScenarioCase) -> Result<(), OracleError> {
    validate_common(path, &case.common())?;
    routing_scenarios::validate_in_case(path, &case.scenario)
}

pub(super) struct CommonFields<'input> {
    pub(super) name: &'input str,
    pub(super) config: &'input str,
    pub(super) env: &'input BTreeMap<String, String>,
    pub(super) native_feature_eligibility: bool,
    pub(super) database: Option<&'input DatabaseFixture>,
    pub(super) upstream: &'input [UpstreamAnswer],
}

pub(super) fn validate_common(path: &Path, fields: &CommonFields<'_>) -> Result<(), OracleError> {
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
    if super::config_inspection::requests_license(path, fields.config)? {
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
