use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::request_destination::RequestData;
use crate::api::request_model_overrides::catalog::DeploymentCatalogView;
use crate::api::request_model_overrides::diagnostics::OverrideDiagnostic;
use crate::api::request_model_overrides::error::RequestModelOverridesError;
use crate::api::request_model_overrides::requested_model;
use crate::api::request_model_overrides::warnings::WarnOnce;
use crate::api::request_model_overrides::{MODEL_FIELD, OverridesPrincipal};

// the router and the warn-once set the stale-alias branches read
pub(crate) struct AliasInputs<'a> {
    pub(crate) catalog: Option<&'a dyn DeploymentCatalogView>,
    pub(crate) team_stale_alias_bypass: bool,
    pub(crate) warnings: &'a WarnOnce,
}

impl AliasInputs<'_> {
    // a deleted deployment's alias never applies, and a bypassed stale alias never wins over the team's siblings
    fn keeps_requested_model(
        &self,
        principal: &OverridesPrincipal,
        model: &str,
        target: &str,
        diagnostics: &mut Vec<OverrideDiagnostic>,
    ) -> bool {
        let Some(team_id) = principal.team_id.as_deref() else {
            return false;
        };
        let prefix = format!("model_name_{team_id}_");
        if !target.starts_with(&prefix) {
            return false;
        }
        let Some(catalog) = self.catalog else {
            return false;
        };
        if !catalog.has_model_group(target) {
            if self
                .warnings
                .first_seen(format!("deleted:{team_id}:{model}:{target}"))
            {
                diagnostics.push(OverrideDiagnostic::StaleAliasMissingDeployment {
                    model: model.to_owned(),
                    team_id: team_id.to_owned(),
                    aliased_target: target.to_owned(),
                });
            }
            return true;
        }
        if !catalog.has_team_model_group(team_id, model) {
            return false;
        }
        if self.team_stale_alias_bypass {
            return true;
        }
        if self
            .warnings
            .first_seen(format!("{team_id}:{model}:{target}"))
        {
            diagnostics.push(OverrideDiagnostic::StaleAliasSiblingDeployment {
                model: model.to_owned(),
                team_id: team_id.to_owned(),
            });
        }
        false
    }
}

// _update_model_if_team_alias_exists: the team alias rewrites the name the caller sent
fn apply_team_alias(
    data: &mut RequestData,
    model: &str,
    principal: &OverridesPrincipal,
    inputs: &AliasInputs<'_>,
    diagnostics: &mut Vec<OverrideDiagnostic>,
) {
    let Some(target) = principal.team_model_aliases.get(model) else {
        return;
    };
    if inputs.keeps_requested_model(principal, model, target, diagnostics) {
        return;
    }
    data.insert(MODEL_FIELD.to_owned(), Json::Text(target.clone()));
}

// _update_model_if_key_alias_exists runs on the name the team alias left behind
fn apply_key_alias(data: &mut RequestData, model: &str, aliases: &IndexMap<String, String>) {
    let Some(target) = aliases.get(model) else {
        return;
    };
    data.insert(MODEL_FIELD.to_owned(), Json::Text(target.clone()));
}

pub(crate) fn apply_model_aliases(
    data: &mut RequestData,
    principal: &OverridesPrincipal,
    inputs: &AliasInputs<'_>,
    diagnostics: &mut Vec<OverrideDiagnostic>,
) -> Result<(), RequestModelOverridesError> {
    let Some(model) = requested_model(data)? else {
        return Ok(());
    };
    apply_team_alias(data, &model, principal, inputs, diagnostics);
    let Some(aliased) = requested_model(data)? else {
        return Ok(());
    };
    apply_key_alias(data, &aliased, &principal.aliases);
    Ok(())
}
