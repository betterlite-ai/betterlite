use std::collections::BTreeMap;

use betterlite_auth::auth_model_patterns::decision::ModelAccessContext;
use betterlite_auth::auth_model_patterns::denial::ModelAccessDecision;
use betterlite_auth::auth_model_patterns::pattern::CatalogProviderInference;
use betterlite_auth::auth_model_patterns::user_model::can_user_call_model;
use betterlite_auth::auth_request_models::{RequestedModels, format_model_candidates};
use betterlite_config::sections::database_reencryption::DEFAULT_RECURSE_DEPTH;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_models::catalog::BundledCatalog;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::{Deserialize, Serialize};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    checks: Vec<Check>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Check {
    model: Requested,
    user: Option<LiteLLMUserTable>,
    #[serde(default)]
    key_aliases: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum Requested {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Serialize)]
struct CheckResult {
    allowed: bool,
    error: Option<DenialView>,
}

#[derive(Debug, Serialize)]
struct DenialView {
    #[serde(rename = "type")]
    error_type: String,
    param: Option<String>,
    code: String,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut bundled = BundledCatalog::default();
        let catalog = bundled.get();
        let inference = CatalogProviderInference {
            catalog: Some(catalog.as_ref()),
        };
        let empty_aliases = BTreeMap::new();
        let context = ModelAccessContext {
            view: None,
            global_model_aliases: &empty_aliases,
            compaction: None,
            provider: &inference,
            max_recurse_depth: DEFAULT_RECURSE_DEPTH,
        };
        let mut results = Vec::with_capacity(input.checks.len());
        for check in &input.checks {
            let requested = match &check.model {
                Requested::One(name) => RequestedModels::One(name.clone()),
                Requested::Many(names) => format_model_candidates(names.clone()),
            };
            let decision = can_user_call_model(
                &requested,
                check.user.as_ref(),
                &check.key_aliases,
                &context,
            );
            results.push(result_view(decision)?);
        }
        Ok(serde_json::to_value(results)?)
    }
}

fn result_view(decision: ModelAccessDecision) -> Result<CheckResult, Box<HelperError>> {
    match decision {
        ModelAccessDecision::Allowed => Ok(CheckResult {
            allowed: true,
            error: None,
        }),
        ModelAccessDecision::Denied(denial) => {
            let exception = denial.as_proxy_exception();
            Ok(CheckResult {
                allowed: false,
                error: Some(DenialView {
                    error_type: exception.error_type.clone(),
                    param: exception.param.clone(),
                    code: exception.code.clone(),
                }),
            })
        }
        ModelAccessDecision::RecurseDepthExceeded { model } => Err(Box::new(HelperError {
            class: "Exception".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: format!(
                "Unable to parse model, max fallback depth exceeded - received model: {model}"
            ),
        })),
    }
}
