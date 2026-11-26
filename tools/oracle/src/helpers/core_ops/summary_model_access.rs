mod diagnostics;
mod journal;
mod real;
mod scripted;

use std::collections::BTreeMap;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::sections::Section;
use betterlite_config::sections::summary_model_access::Raw;
use betterlite_core::api::summary_model_access::access::{
    ProxyGlobals, check_summary_model_access,
};
use betterlite_core::api::summary_model_access::checks::SummaryModelChecks;
use betterlite_core::api::summary_model_access::repositories::SummaryAccessRepositories;
use betterlite_core::api::summary_model_access::settings::{
    read_summary_max_tokens, read_summary_model_setting,
};
use betterlite_db::budget_permission_rows::JsonData;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::runtime::Builder;
use tracing::subscriber::set_default;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::registry;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use diagnostics::{DiagnosticRecord, Diagnostics};
use journal::{CallRecord, Journal};
use real::RealChecks;
use scripted::{ScriptOutcome, Scripted, ScriptedLookup};

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Settings {
        scenarios: Vec<SettingsScenario>,
    },
    Access {
        summary_model: String,
        scenarios: Vec<AccessScenario>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsScenario {
    // null means a process without the proxy module, where each reader takes its fallback
    general_settings: Option<BTreeMap<String, JsonData>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccessScenario {
    principal: Option<BTreeMap<String, JsonData>>,
    // false means a process whose proxy_server import fails, where the check allows every summary model
    proxy: bool,
    // prisma_client is not None
    database: bool,
    checks: CheckMode,
    outcomes: Vec<ScriptOutcome>,
    user: LookupScript,
    project: LookupScript,
    membership: LookupScript,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CheckMode {
    Scripted,
    Real,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum LookupScript {
    Missing,
    Failed,
    Found { row: JsonData },
}

#[derive(Debug, Serialize)]
struct AccessOutcome {
    allowed: bool,
    calls: Vec<CallRecord>,
    diagnostics: Vec<DiagnosticRecord>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Settings { scenarios } => {
                let mut outcomes = Vec::with_capacity(scenarios.len());
                for scenario in scenarios {
                    let (model, max_tokens) = settings_outcome(scenario)?;
                    outcomes.push(json!({ "model": model, "max_tokens": max_tokens }));
                }
                Ok(json!({ "outcomes": outcomes }))
            }
            Input::Access {
                summary_model,
                scenarios,
            } => {
                let runtime = Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .or_value_error()?;
                let mut outcomes = Vec::with_capacity(scenarios.len());
                for scenario in scenarios {
                    outcomes.push(runtime.block_on(access_outcome(summary_model, scenario))?);
                }
                Ok(json!({ "outcomes": outcomes }))
            }
        }
    }
}

// the two readers over the settings the proxy decoded, or their fallbacks without a proxy
fn settings_outcome(
    scenario: &SettingsScenario,
) -> Result<(Option<String>, u64), Box<HelperError>> {
    let Some(general_settings) = &scenario.general_settings else {
        return Ok((
            read_summary_model_setting(None),
            read_summary_max_tokens(None),
        ));
    };
    // the section's own view of general_settings, so the adapter holds no second YAML decoder
    let document = json!({ "general_settings": general_settings });
    let raw: Raw = serde_json::from_value(document).or_value_error()?;
    let settings = Section::convert(raw, &BTreeMap::<String, String>::new()).or_config_error()?;
    Ok((
        read_summary_model_setting(Some(&settings)),
        read_summary_max_tokens(Some(&settings)),
    ))
}

async fn access_outcome(
    summary_model: &str,
    scenario: &AccessScenario,
) -> Result<AccessOutcome, Box<HelperError>> {
    let principal = principal_of(scenario)?;
    let journal = Journal::default();
    let diagnostics = Diagnostics::default();
    let guard = set_default(registry().with(diagnostics.clone()));
    let allowed = checked(summary_model, scenario, principal.as_ref(), &journal).await?;
    drop(guard);
    Ok(AccessOutcome {
        allowed,
        calls: journal.records(),
        diagnostics: diagnostics.lines(),
    })
}

async fn checked(
    summary_model: &str,
    scenario: &AccessScenario,
    principal: Option<&UserAPIKeyAuth>,
    journal: &Journal,
) -> Result<bool, Box<HelperError>> {
    let scripted = Scripted::new(
        journal,
        &scenario.outcomes,
        lookup(&scenario.user)?,
        lookup(&scenario.project)?,
        lookup(&scenario.membership)?,
    );
    if !scenario.proxy {
        // the absent proxy still names the two generic parameters the check is instantiated with
        let absent: Option<&ProxyGlobals<'_, Scripted<'_>, Scripted<'_>>> = None;
        return check_summary_model_access(principal, summary_model, absent)
            .await
            .or_value_error();
    }
    match scenario.checks {
        CheckMode::Real => {
            let checks = RealChecks { journal };
            with_checks(principal, summary_model, scenario, &checks, &scripted).await
        }
        CheckMode::Scripted => {
            with_checks(principal, summary_model, scenario, &scripted, &scripted).await
        }
    }
}

async fn with_checks<Checks, Repositories>(
    principal: Option<&UserAPIKeyAuth>,
    summary_model: &str,
    scenario: &AccessScenario,
    checks: &Checks,
    repositories: &Repositories,
) -> Result<bool, Box<HelperError>>
where
    Checks: SummaryModelChecks,
    Repositories: SummaryAccessRepositories,
{
    let globals = ProxyGlobals {
        database: scenario.database,
        router: None,
        checks,
        repositories,
    };
    check_summary_model_access(principal, summary_model, Some(&globals))
        .await
        .or_value_error()
}

fn principal_of(scenario: &AccessScenario) -> Result<Option<UserAPIKeyAuth>, Box<HelperError>> {
    scenario
        .principal
        .clone()
        .map(|fields| UserAPIKeyAuth::construct(AuthInput::Dict(fields)))
        .transpose()
        .or_value_error()
}

fn lookup<Row: DeserializeOwned>(
    script: &LookupScript,
) -> Result<ScriptedLookup<Row>, Box<HelperError>> {
    match script {
        LookupScript::Missing => Ok(ScriptedLookup::Missing),
        LookupScript::Failed => Ok(ScriptedLookup::Failed),
        LookupScript::Found { row } => {
            let text = serde_json::to_string(row).or_value_error()?;
            let decoded = serde_json::from_str(&text).or_value_error()?;
            Ok(ScriptedLookup::Found(decoded))
        }
    }
}
