use std::collections::BTreeMap;

use betterlite_auth::auth_model_patterns::decision::{ModelAccessContext, can_object_call_model};
use betterlite_auth::auth_model_patterns::denial::ModelAccessDecision;
use betterlite_auth::auth_model_patterns::grant::{
    ModelGrantPattern, key_model_aliases, resolve_key_models,
};
use betterlite_auth::auth_model_patterns::pattern::{CatalogProviderInference, ProviderInference};
use betterlite_auth::auth_model_patterns::view::{
    Deployment, DeploymentView, ModelAccessView, ModelGroupAlias,
};
use betterlite_auth::auth_request_models::RequestedModels;
use betterlite_config::sections::database_reencryption::DEFAULT_RECURSE_DEPTH;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_error::proxy_error_types::ModelAccessObject;
use betterlite_models::catalog::BundledCatalog;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::helpers::{Helper, HelperError, HelperResult};

const FORBIDDEN: i64 = 403;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Step {
    object_kind: ObjectKind,
    model: String,
    models: Vec<String>,
    team_id: Option<String>,
    team_models: Vec<String>,
    team_model_aliases: BTreeMap<String, String>,
    key_aliases: Option<BTreeMap<String, JsonData>>,
    access_group_ids: Vec<String>,
    deployments: Vec<DeploymentInput>,
    model_group_alias: BTreeMap<String, AliasInput>,
    global_aliases: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ObjectKind {
    #[default]
    Key,
    Team,
    Org,
}

impl ObjectKind {
    const fn model_access_object(self) -> ModelAccessObject {
        match self {
            Self::Key => ModelAccessObject::Key,
            Self::Team => ModelAccessObject::Team,
            Self::Org => ModelAccessObject::Org,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeploymentInput {
    model_name: String,
    #[serde(default)]
    access_groups: Vec<String>,
    #[serde(default)]
    team_id: Option<String>,
    #[serde(default)]
    team_public_model_name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum AliasInput {
    Target(String),
    Item {
        model: String,
        #[serde(default)]
        hidden: bool,
    },
}

#[derive(Debug, Serialize)]
struct StepResult {
    allowed: bool,
    error: Option<ErrorOut>,
    key_aliases: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Serialize)]
struct ErrorOut {
    error_class: &'static str,
    status: i64,
    error_type: String,
    param: Option<String>,
    code: String,
    message: String,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut bundled = BundledCatalog::default();
        let catalog = bundled.get();
        let inference = CatalogProviderInference {
            catalog: Some(catalog.as_ref()),
        };
        let results = input
            .steps
            .iter()
            .map(|step| run_step(step, &inference))
            .collect::<Result<Vec<StepResult>, Box<HelperError>>>()?;
        Ok(serde_json::to_value(results)?)
    }
}

fn run_step(
    step: &Step,
    inference: &dyn ProviderInference,
) -> Result<StepResult, Box<HelperError>> {
    let view = build_view(step);
    let team_id = match step.object_kind {
        ObjectKind::Org => None,
        ObjectKind::Key | ObjectKind::Team => step.team_id.as_deref(),
    };
    let key_aliases = step
        .key_aliases
        .as_ref()
        .filter(|aliases| !aliases.is_empty())
        .map(key_model_aliases);
    let grant = build_grant(step, team_id, key_aliases.as_ref());
    let context = ModelAccessContext {
        view: view.as_deref(),
        global_model_aliases: &step.global_aliases,
        compaction: None,
        provider: inference,
        max_recurse_depth: DEFAULT_RECURSE_DEPTH,
    };
    // the helper injects no access-group store, so the wrapper's group fallback grants nothing and the core decision stands
    let requested = RequestedModels::One(step.model.clone());
    let decision = can_object_call_model(
        &requested,
        &grant,
        step.object_kind.model_access_object(),
        &context,
        0,
    );
    let error = error_out(&decision)?;
    Ok(StepResult {
        allowed: error.is_none(),
        error,
        key_aliases,
    })
}

fn build_grant(
    step: &Step,
    team_id: Option<&str>,
    key_aliases: Option<&BTreeMap<String, String>>,
) -> ModelGrantPattern {
    let models = match step.object_kind {
        ObjectKind::Key => resolve_key_models(&step.models, &step.team_models, team_id),
        // a team object always carries a team_id; without one can_team_access_model reads no models
        ObjectKind::Team if team_id.is_none() => Vec::new(),
        ObjectKind::Team | ObjectKind::Org => step.models.clone(),
    };
    ModelGrantPattern {
        models,
        team_model_aliases: step.team_model_aliases.clone(),
        // can_org_access_model takes no key aliases
        key_model_aliases: match step.object_kind {
            ObjectKind::Org => BTreeMap::new(),
            ObjectKind::Key | ObjectKind::Team => key_aliases.cloned().unwrap_or_default(),
        },
        team_id: team_id.map(str::to_owned),
    }
}

fn error_out(decision: &ModelAccessDecision) -> Result<Option<ErrorOut>, Box<HelperError>> {
    match decision {
        ModelAccessDecision::Allowed => Ok(None),
        ModelAccessDecision::Denied(denial) => {
            let exception = denial.as_proxy_exception();
            Ok(Some(ErrorOut {
                error_class: "ModelAccessDeniedProxyException",
                status: FORBIDDEN,
                error_type: exception.error_type.clone(),
                param: exception.param.clone(),
                code: exception.code.clone(),
                message: exception.message.clone(),
            }))
        }
        ModelAccessDecision::RecurseDepthExceeded { model } => {
            let message = format!(
                "Unable to parse model, max fallback depth exceeded - received model: {model}"
            );
            Err(Box::new(HelperError {
                class: "Exception".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message,
            }))
        }
    }
}

// the oracle builds a Router only when there is a deployment or an alias to serve
fn build_view(step: &Step) -> Option<Box<dyn ModelAccessView>> {
    if step.deployments.is_empty() && step.model_group_alias.is_empty() {
        return None;
    }
    let deployments = step
        .deployments
        .iter()
        .map(|deployment| Deployment {
            model_name: deployment.model_name.clone(),
            access_groups: deployment.access_groups.clone(),
            team_id: deployment.team_id.clone(),
            team_public_model_name: deployment.team_public_model_name.clone(),
        })
        .collect();
    let mut model_group_alias = IndexMap::new();
    for (alias, entry) in &step.model_group_alias {
        let (model, hidden) = match entry {
            AliasInput::Target(target) => (target.clone(), false),
            AliasInput::Item { model, hidden } => (model.clone(), *hidden),
        };
        model_group_alias.insert(alias.clone(), ModelGroupAlias { model, hidden });
    }
    Some(Box::new(DeploymentView {
        deployments,
        model_group_alias,
    }))
}
