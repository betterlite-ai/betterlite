use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use crate::OracleError;
use crate::case::Case;
mod tests;
pub(crate) const RELEASE_FIXTURE_KEY: &str = "latest_release_fixture";
const STARTUP_HOOK: &str = "startup:set_release_url";
const STARTUP_VARIABLE: &str = "RELEASE_ORACLE_URL";
const STARTUP_HOOKS_VARIABLE: &str = "LITELLM_WORKER_STARTUP_HOOKS";
const PYTHON_PATH_VARIABLE: &str = "PYTHONPATH";
const DESTINATION_VARIABLE: &str = "BETTERLITE_LATEST_RELEASE_URL";
const FIXTURE_DIRECTORY: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/release-oracle");
const NATIVE_CONFLICT: &str =
    "latest_release_fixture cannot be combined with native_feature_eligibility";
const CONFIG_CONFLICT: &str = "latest_release_fixture cannot be combined with a config that sets BETTERLITE_LATEST_RELEASE_URL";

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReleaseFixture {
    Upstream,
}

pub(crate) enum PreparedCase<'text> {
    Plain(&'text str),
    Fixture {
        text: String,
        fixture: ReleaseFixture,
    },
}

impl PreparedCase<'_> {
    pub(crate) fn text(&self) -> &str {
        match self {
            Self::Plain(text) => text,
            Self::Fixture { text, .. } => text,
        }
    }

    pub(crate) fn fixture(&self) -> Option<ReleaseFixture> {
        match self {
            Self::Plain(_) => None,
            Self::Fixture { fixture, .. } => Some(*fixture),
        }
    }
}

impl ReleaseFixture {
    pub(crate) fn apply(self, case: &mut Case) {
        case.env
            .insert(STARTUP_VARIABLE.to_owned(), self.destination().to_owned());
        case.env.insert(
            STARTUP_HOOKS_VARIABLE.to_owned(),
            self.startup_hook().to_owned(),
        );
        case.env.insert(
            PYTHON_PATH_VARIABLE.to_owned(),
            FIXTURE_DIRECTORY.to_owned(),
        );
        let section = format!(
            "environment_variables:\n  {DESTINATION_VARIABLE}: \"{}\"\n",
            self.destination()
        );
        case.config.push('\n');
        case.config.push_str(&section);
    }

    fn destination(self) -> &'static str {
        match self {
            Self::Upstream => "{{upstream}}",
        }
    }

    fn startup_hook(self) -> &'static str {
        match self {
            Self::Upstream => STARTUP_HOOK,
        }
    }
}

#[derive(Deserialize)]
struct FixtureHost {
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    native_feature_eligibility: bool,
    #[serde(default)]
    config: String,
}

pub(crate) fn prepare<'text>(
    path: &Path,
    text: &'text str,
) -> Result<PreparedCase<'text>, OracleError> {
    if !text.contains(RELEASE_FIXTURE_KEY) {
        return Ok(PreparedCase::Plain(text));
    }
    let mut document: Value =
        serde_saphyr::from_str(text).map_err(|source| OracleError::CaseParse {
            path: path.to_owned(),
            source: Box::new(source),
        })?;
    let Some(object) = document.as_object_mut() else {
        return Ok(PreparedCase::Plain(text));
    };
    let Some(fixture) = object.remove(RELEASE_FIXTURE_KEY) else {
        return Ok(PreparedCase::Plain(text));
    };
    let fixture: ReleaseFixture = serde_json::from_value(fixture)
        .map_err(|source| invalid(path, &format!("{RELEASE_FIXTURE_KEY}: {source}")))?;
    let host: FixtureHost = serde_json::from_value(document.clone()).map_err(|source| {
        invalid(
            path,
            &format!("the case around {RELEASE_FIXTURE_KEY} does not read: {source}"),
        )
    })?;
    validate(path, &host)?;
    let text = serde_json::to_string(&document).map_err(OracleError::Json)?;
    Ok(PreparedCase::Fixture { text, fixture })
}

fn validate(path: &Path, host: &FixtureHost) -> Result<(), OracleError> {
    if host.native_feature_eligibility {
        return Err(invalid(path, NATIVE_CONFLICT));
    }
    for variable in [
        STARTUP_HOOKS_VARIABLE,
        PYTHON_PATH_VARIABLE,
        STARTUP_VARIABLE,
    ] {
        if host.env.contains_key(variable) {
            return Err(invalid(
                path,
                &format!("{RELEASE_FIXTURE_KEY} cannot be combined with env {variable}"),
            ));
        }
    }
    if host.config.contains(DESTINATION_VARIABLE) {
        return Err(invalid(path, CONFIG_CONFLICT));
    }
    Ok(())
}

fn invalid(path: &Path, message: &str) -> OracleError {
    let message = message.to_owned();
    OracleError::CaseInvalid {
        path: path.to_owned(),
        message,
    }
}
