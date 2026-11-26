use std::collections::BTreeMap;
use std::future::Future;

use betterlite_auth::auth_model_patterns::denial::model_access_object_name;
use betterlite_auth::auth_model_patterns::grant::key_model_aliases;
use betterlite_auth::auth_model_patterns::view::ModelAccessView;
use betterlite_auth::auth_read::membership::member_allowed_models;
use betterlite_auth::entity_model_access::grant::text_models;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_error::proxy_error_types::ModelAccessObject;
use serde::Deserialize;

use crate::api::summary_model_access::checks::{ObjectCall, SummaryModelChecks};
use crate::api::summary_model_access::error::{PolicyFailure, SummaryAccessError};
use crate::api::summary_model_access::repositories::SummaryAccessRepositories;

// the proxy globals compact.py imports inside the check: without them the check allows every summary model
pub struct ProxyGlobals<'globals, Checks, Repositories> {
    // prisma_client is not None
    pub database: bool,
    pub router: Option<&'globals dyn ModelAccessView>,
    pub checks: &'globals Checks,
    pub repositories: &'globals Repositories,
}

pub async fn check_summary_model_access<Checks, Repositories>(
    principal: Option<&UserAPIKeyAuth>,
    summary_model: &str,
    proxy: Option<&ProxyGlobals<'_, Checks, Repositories>>,
) -> Result<bool, SummaryAccessError>
where
    Checks: SummaryModelChecks,
    Repositories: SummaryAccessRepositories,
{
    let (Some(auth), Some(globals)) = (principal, proxy) else {
        return Ok(true);
    };
    let fields: AccessFields = serde_json::to_value(auth)
        .and_then(serde_json::from_value)
        .map_err(SummaryAccessError::Principal)?;
    let scope = Scope {
        model: summary_model,
        fields: &fields,
        database: globals.database,
        router: globals.router,
        checks: globals.checks,
        repositories: globals.repositories,
    };
    // source order, stopping at the first denial so a denied scope prevents the later lookups
    let denied =
        scope.key_and_team() || scope.user().await || scope.project().await || scope.member().await;
    Ok(!denied)
}

// the UserAPIKeyAuth fields _check_summary_model_access reads, taken from the principal's serde form
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct AccessFields {
    models: Option<Vec<JsonData>>,
    team_models: Option<Vec<JsonData>>,
    team_model_aliases: Option<BTreeMap<String, JsonData>>,
    team_id: Option<String>,
    user_id: Option<String>,
    project_id: Option<String>,
}

struct Scope<'scope, 'globals, Checks, Repositories> {
    model: &'scope str,
    fields: &'scope AccessFields,
    database: bool,
    router: Option<&'globals dyn ModelAccessView>,
    checks: &'globals Checks,
    repositories: &'globals Repositories,
}

impl<Checks, Repositories> Scope<'_, '_, Checks, Repositories>
where
    Checks: SummaryModelChecks,
    Repositories: SummaryAccessRepositories,
{
    // empty key and team allowlists skip their check, and an empty-string id still looks its object up
    fn key_and_team(&self) -> bool {
        let key_models = text_models(self.fields.models.as_deref().unwrap_or_default());
        let team_models = text_models(self.fields.team_models.as_deref().unwrap_or_default());
        let alias_values = self.fields.team_model_aliases.clone().unwrap_or_default();
        let aliases = key_model_aliases(&alias_values);
        let checks = [
            (ModelAccessObject::Key, &key_models),
            (ModelAccessObject::Team, &team_models),
        ];
        for (object_type, models) in checks {
            if models.is_empty() {
                continue;
            }
            let call = ObjectCall {
                model: self.model,
                models,
                team_model_aliases: &aliases,
                team_id: self.fields.team_id.as_deref(),
                object_type,
            };
            let outcome = self.checks.object_call(self.router, &call);
            let scope = model_access_object_name(object_type);
            if self.denies(outcome, scope) {
                return true;
            }
        }
        false
    }

    // a failed read skips the scope, with the exception's text logged as compact.py does
    async fn loaded<Row, Lookup>(&self, scope: &str, lookup: Lookup) -> Option<Row>
    where
        Lookup: Future<Output = Result<Option<Row>, Repositories::Failure>> + Send,
    {
        let model = self.model;
        match lookup.await {
            Ok(row) => row,
            Err(failure) => {
                tracing::debug!(
                    "compact_20260112: {scope} object lookup failed for summary_model={model} access \
                     check; skipping {scope}-level scope: {failure}"
                );
                None
            }
        }
    }

    // a failed user read skips the user scope, and the lookup needs a database and an id, empty string included
    async fn user(&self) -> bool {
        let Some(user_id) = self.fields.user_id.as_deref().filter(|_| self.database) else {
            return false;
        };
        // the summary request reads the row without ever upserting it
        let loaded = self
            .loaded("user", self.repositories.user(user_id, false))
            .await;
        let Some(user) = loaded else {
            return false;
        };
        self.denies(
            self.checks.user_call(self.router, self.model, &user),
            "user",
        )
    }

    // a missing project or an empty project allowlist skips the project scope
    async fn project(&self) -> bool {
        let Some(project_id) = self.fields.project_id.as_deref().filter(|_| self.database) else {
            return false;
        };
        let Some(project) = self
            .loaded("project", self.repositories.project(project_id))
            .await
        else {
            return false;
        };
        if project.models.is_empty() {
            return false;
        }
        self.denies(
            self.checks.project_call(self.router, self.model, &project),
            "project",
        )
    }

    // a failed membership read denies, because a member whose limits cannot be read must not lose them
    async fn member(&self) -> bool {
        let model = self.model;
        let (Some(user_id), Some(team_id)) = (
            self.fields.user_id.as_deref().filter(|_| self.database),
            self.fields.team_id.as_deref().filter(|_| self.database),
        ) else {
            return false;
        };
        let loaded = match self.repositories.membership(user_id, team_id).await {
            Ok(row) => row,
            Err(failure) => {
                tracing::warn!(
                    "compact_20260112: team membership lookup failed for summary_model={model} \
                     access check; denying access: {failure}"
                );
                return true;
            }
        };
        let Some(membership) = loaded else {
            return false;
        };
        let allowed_models = member_allowed_models(&membership);
        if allowed_models.is_empty() {
            return false;
        }
        // the member check is a team check over the membership budget's allowlist, aliases and team id included
        let alias_values = self.fields.team_model_aliases.clone().unwrap_or_default();
        let aliases = key_model_aliases(&alias_values);
        let call = ObjectCall {
            model,
            models: allowed_models,
            team_model_aliases: &aliases,
            team_id: Some(team_id),
            object_type: ModelAccessObject::Team,
        };
        let outcome = self.checks.object_call(self.router, &call);
        self.denies(outcome, "member")
    }

    // a ProxyException is a silent denial, and any other failure logs LiteLLM's warning before denying
    fn denies(&self, outcome: Result<(), PolicyFailure<Checks::Failure>>, scope: &str) -> bool {
        let model = self.model;
        match outcome {
            Ok(()) => false,
            Err(PolicyFailure::Denied) => true,
            Err(PolicyFailure::Unexpected(failure)) => {
                tracing::warn!(
                    "compact_20260112: unexpected error during {scope}-level access check for \
                     summary_model={model}; denying access: {failure}"
                );
                true
            }
        }
    }
}
