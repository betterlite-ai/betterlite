use std::collections::BTreeSet;

use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::judge_primitives::target::{JudgeTarget, JudgeVia};
use betterlite_models::resolution::{ProviderIdentity, ResolutionRequest, resolve};
use betterlite_secret_managers::secret_helpers::manager_read_is_consulted;
use secrecy::ExposeSecret as _;

use crate::judge_targets::dispatcher::JudgeDispatcher;
use crate::judge_targets::judge_target;
use crate::judge_targets::router::JudgeRouter;
use crate::router_dependencies::StrategyRouterDependencyRole;
use crate::router_dependencies::dependencies::strategy_router_dependencies;
use crate::shadow_eval_validation::error::ShadowEvalValidationError;
use crate::shadow_eval_validation::request::StartShadowEvalRequest;
use crate::shadow_eval_validation::start::StartContext;

const ANTHROPIC_API_KEY: &str = "ANTHROPIC_API_KEY";
const ANTHROPIC_AUTH_TOKEN: &str = "ANTHROPIC_AUTH_TOKEN";

fn model_target(context: &StartContext<'_>, model: &str, team_id: Option<&str>) -> JudgeTarget {
    let router: Option<&dyn JudgeRouter> = context
        .router
        .as_ref()
        .map(|router| -> &dyn JudgeRouter { router });
    let dispatcher = JudgeDispatcher {
        router,
        sdk: context.sdk,
        cache: context.cache,
        context: context.resolution,
    };
    judge_target(&dispatcher, model, team_id)
}

// _validate_plain_model: the judge and a reverse baseline must resolve once per sampled team and answer from a fixed arm.
pub fn validate_plain_model(
    context: &StartContext<'_>,
    model: &str,
    field: &'static str,
    team_ids: &[Option<String>],
) -> Result<(), ShadowEvalValidationError> {
    let configured_router = context
        .router
        .as_ref()
        .is_some_and(|router| router.has_strategy_deployment(model));
    if configured_router {
        return Err(ShadowEvalValidationError::AutoRouterModel {
            field,
            model: model.to_owned(),
        });
    }
    let mut unreachable: Vec<Option<String>> = Vec::new();
    let mut sdk_teams: Vec<Option<String>> = Vec::new();
    for team_id in team_ids {
        match model_target(context, model, team_id.as_deref()).via {
            JudgeVia::Router => {}
            JudgeVia::Sdk => sdk_teams.push(team_id.clone()),
            JudgeVia::Nothing => unreachable.push(team_id.clone()),
        }
    }
    if !unreachable.is_empty() {
        let teams = for_teams(&unreachable);
        return Err(ShadowEvalValidationError::UnreachableModel {
            message: format!(
                "{field} '{model}' is not a configured or provider-qualified public model{teams}"
            ),
        });
    }
    if sdk_teams.is_empty() || !missing_anthropic_credentials(model, context) {
        return Ok(());
    }
    let teams = for_teams(&sdk_teams);
    Err(ShadowEvalValidationError::MissingAnthropicCredentials {
        message: format!(
            "{field} '{model}' needs ANTHROPIC_API_KEY or ANTHROPIC_AUTH_TOKEN{teams}"
        ),
    })
}

fn for_teams(team_ids: &[Option<String>]) -> String {
    let mut named: Vec<&str> = team_ids.iter().filter_map(Option::as_deref).collect();
    if named.is_empty() {
        return String::new();
    }
    named.sort_unstable();
    format!(" for team {}", named.join(", "))
}

fn missing_anthropic_credentials(model: &str, context: &StartContext<'_>) -> bool {
    let request = ResolutionRequest {
        model,
        custom_llm_provider: None,
        api_base: None,
        api_key: None,
    };
    let Ok(resolved) = resolve(&request, context.resolution) else {
        return false;
    };
    if resolved.provider != ProviderIdentity::Known(ProviderId::Anthropic) {
        return false;
    }
    let sources = &context.credentials;
    let configured = [
        sources.anthropic_key,
        sources.api_key,
        sources.settings.api_key.as_ref(),
        sources.settings.auth_token.as_ref(),
    ]
    .into_iter()
    .any(|secret| secret.is_some_and(|value| !value.expose_secret().is_empty()));
    if configured {
        return false;
    }
    !(manager_read_is_consulted(sources.secret_state, ANTHROPIC_API_KEY)
        || manager_read_is_consulted(sources.secret_state, ANTHROPIC_AUTH_TOKEN))
}

// _validate_model_scope: reject a scope naming a model no request on this proxy could carry, including team-public names.
pub fn validate_model_scope(
    context: &StartContext<'_>,
    models: &[String],
) -> Result<(), ShadowEvalValidationError> {
    let public_names = context
        .router
        .as_ref()
        .map_or_else(BTreeSet::new, |router| {
            router.catalog.team_public_model_names()
        });
    let unreachable: Vec<&String> = models
        .iter()
        .filter(|model| {
            model_target(context, model, None).via == JudgeVia::Nothing
                && !public_names.contains(model.as_str())
        })
        .collect();
    if unreachable.is_empty() {
        return Ok(());
    }
    let listed = unreachable
        .iter()
        .map(|model| format!("'{model}'"))
        .collect::<Vec<String>>()
        .join(", ");
    Err(ShadowEvalValidationError::UnservedModels { models: listed })
}

pub fn router_arm_models(
    context: &StartContext<'_>,
    router_name: &str,
) -> Result<Vec<(StrategyRouterDependencyRole, String)>, ShadowEvalValidationError> {
    let Some(router) = context.router.as_ref() else {
        return Ok(Vec::new());
    };
    let rows = router
        .lookup()
        .get_model_list(Some(router_name), None)
        .map_err(ShadowEvalValidationError::RouterArms)?;
    let mut seen = BTreeSet::new();
    Ok(rows
        .iter()
        .flat_map(|row| strategy_router_dependencies(&row.litellm_params))
        .filter(|dependency| {
            dependency.role == StrategyRouterDependencyRole::Tier
                || dependency.role == StrategyRouterDependencyRole::Default
        })
        .filter(|dependency| seen.insert((dependency.role, dependency.model_name.clone())))
        .map(|dependency| (dependency.role, dependency.model_name))
        .collect())
}

// _reject_judge_collision: a judge scoring its own arm measures the judge, not the models.
pub fn reject_judge_collision(
    context: &StartContext<'_>,
    request: &StartShadowEvalRequest,
    team_ids: &[Option<String>],
) -> Result<(), ShadowEvalValidationError> {
    let mut arms: Vec<(String, String)> = Vec::new();
    for name in &request.router_names {
        for (role, model) in router_arm_models(context, name)? {
            arms.push((role.name().to_owned(), model));
        }
    }
    if let Some(baseline) = &request.baseline_model {
        arms.push(("baseline".to_owned(), baseline.clone()));
    }
    let mut seen = BTreeSet::new();
    let mut collisions: Vec<(String, String)> = Vec::new();
    for team_id in team_ids {
        let judge = model_target(context, &request.judge_model, team_id.as_deref()).models;
        for (role, model) in &arms {
            let candidate = model_target(context, model, team_id.as_deref());
            let collision = (role.clone(), model.clone());
            if !judge.is_disjoint(&candidate.models) && seen.insert(collision.clone()) {
                collisions.push(collision);
            }
        }
    }
    if collisions.is_empty() {
        return Ok(());
    }
    let listed = collisions
        .iter()
        .map(|(role, model)| format!("{role} model '{model}'"))
        .collect::<Vec<String>>()
        .join(", ");
    Err(ShadowEvalValidationError::JudgeIsCandidate {
        judge: request.judge_model.clone(),
        collisions: listed,
    })
}
