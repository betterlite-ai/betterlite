use std::collections::BTreeMap;

use betterlite_auth::auth_model_patterns::decision::{ModelAccessContext, can_object_call_model};
use betterlite_auth::auth_model_patterns::denial::{ModelAccessDecision, model_access_object_name};
use betterlite_auth::auth_model_patterns::grant::ModelGrantPattern;
use betterlite_auth::auth_model_patterns::pattern::UnresolvedProviderInference;
use betterlite_auth::auth_model_patterns::user_model::can_user_call_model;
use betterlite_auth::auth_model_patterns::view::ModelAccessView;
use betterlite_auth::auth_request_models::RequestedModels;
use betterlite_auth::entity_model_access::error::EntityModelAccessError;
use betterlite_auth::entity_model_access::project::can_project_access_model;
use betterlite_auth::identity_grants::resolver::user_models;
use betterlite_config::sections::database_reencryption::DEFAULT_RECURSE_DEPTH;
use betterlite_core::api::summary_model_access::checks::{ObjectCall, SummaryModelChecks};
use betterlite_core::api::summary_model_access::error::PolicyFailure;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::resource_rows::project::ProjectTable;

use super::journal::{CallRecord, Journal};

// compact.py passes no key or global aliases, and _can_object_call_model starts every scope at depth zero
static NO_ALIASES: BTreeMap<String, String> = BTreeMap::new();
static PROVIDER: UnresolvedProviderInference = UnresolvedProviderInference;

#[derive(Debug, thiserror::Error)]
pub(super) enum RealFailure {
    // _can_object_call_model raises a bare Exception here, so compact.py logs it instead of denying silently
    #[error("Unable to parse model, max fallback depth exceeded - received model: {model}")]
    RecurseDepth { model: String },
    #[error(transparent)]
    Entity(EntityModelAccessError),
}

// the matchers B747, B10051 and B886 provide, over the same scripted repositories
pub(super) struct RealChecks<'run> {
    pub(super) journal: &'run Journal,
}

fn context(router: Option<&dyn ModelAccessView>) -> ModelAccessContext<'_> {
    ModelAccessContext {
        view: router,
        global_model_aliases: &NO_ALIASES,
        compaction: None,
        provider: &PROVIDER,
        max_recurse_depth: DEFAULT_RECURSE_DEPTH,
    }
}

fn decision(granted: ModelAccessDecision) -> Result<(), PolicyFailure<RealFailure>> {
    match granted {
        ModelAccessDecision::Allowed => Ok(()),
        ModelAccessDecision::Denied(_) => Err(PolicyFailure::Denied),
        ModelAccessDecision::RecurseDepthExceeded { model } => {
            Err(PolicyFailure::Unexpected(RealFailure::RecurseDepth {
                model,
            }))
        }
    }
}

impl SummaryModelChecks for RealChecks<'_> {
    type Failure = RealFailure;

    fn object_call(
        &self,
        router: Option<&dyn ModelAccessView>,
        call: &ObjectCall<'_>,
    ) -> Result<(), PolicyFailure<RealFailure>> {
        self.journal.record(CallRecord {
            object_type: Some(model_access_object_name(call.object_type)),
            model: Some(call.model.to_owned()),
            models: call.models.to_vec(),
            team_model_aliases: call.team_model_aliases.clone(),
            team_id: call.team_id.map(str::to_owned),
            ..CallRecord::new("object")
        });
        let grant = ModelGrantPattern {
            models: call.models.to_vec(),
            team_model_aliases: call.team_model_aliases.clone(),
            key_model_aliases: BTreeMap::new(),
            team_id: call.team_id.map(str::to_owned),
        };
        let requested = RequestedModels::One(call.model.to_owned());
        decision(can_object_call_model(
            &requested,
            &grant,
            call.object_type,
            &context(router),
            0,
        ))
    }

    fn user_call(
        &self,
        router: Option<&dyn ModelAccessView>,
        model: &str,
        user: &LiteLLMUserTable,
    ) -> Result<(), PolicyFailure<RealFailure>> {
        self.journal.record(CallRecord {
            model: Some(model.to_owned()),
            models: user_models(user),
            ..CallRecord::new("user")
        });
        decision(can_user_call_model(
            &RequestedModels::One(model.to_owned()),
            Some(user),
            &NO_ALIASES,
            &context(router),
        ))
    }

    fn project_call(
        &self,
        router: Option<&dyn ModelAccessView>,
        model: &str,
        project: &ProjectTable,
    ) -> Result<(), PolicyFailure<RealFailure>> {
        self.journal.record(CallRecord {
            model: Some(model.to_owned()),
            models: project.models.clone(),
            ..CallRecord::new("project")
        });
        let requested = RequestedModels::One(model.to_owned());
        let granted =
            can_project_access_model(&requested, Some(project), &NO_ALIASES, &context(router));
        match granted {
            Ok(()) => Ok(()),
            Err(EntityModelAccessError::Denied(_)) => Err(PolicyFailure::Denied),
            Err(EntityModelAccessError::RecurseDepth { model: depth_model }) => {
                Err(PolicyFailure::Unexpected(RealFailure::RecurseDepth {
                    model: depth_model,
                }))
            }
            Err(
                other @ (EntityModelAccessError::Member(_)
                | EntityModelAccessError::Project(_)
                | EntityModelAccessError::TeamRow(_)),
            ) => Err(PolicyFailure::Unexpected(RealFailure::Entity(other))),
        }
    }
}
