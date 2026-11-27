use std::collections::VecDeque;
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_auth::auth_model_patterns::denial::model_access_object_name;
use betterlite_auth::auth_model_patterns::view::ModelAccessView;
use betterlite_auth::identity_grants::resolver::user_models;
use betterlite_core::api::summary_model_access::checks::{ObjectCall, SummaryModelChecks};
use betterlite_core::api::summary_model_access::error::PolicyFailure;
use betterlite_core::api::summary_model_access::repositories::SummaryAccessRepositories;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::resource_rows::project::ProjectTable;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use serde::Deserialize;

use super::journal::{CallRecord, Journal};

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScriptOutcome {
    Allow,
    Deny,
    Broken,
}

// compact.py returns false without logging when a policy check raises a ProxyException
#[derive(Debug, thiserror::Error)]
#[error("broken")]
pub(super) struct BrokenCheck;

// compact.py skips the user and project scopes on a lookup failure and logs the exception's text
#[derive(Debug, thiserror::Error)]
#[error("offline")]
pub(super) struct Offline;

pub(super) enum ScriptedLookup<Row> {
    Missing,
    Failed,
    Found(Row),
}

impl<Row: Clone> ScriptedLookup<Row> {
    fn answer(&self) -> Result<Option<Row>, Offline> {
        match self {
            Self::Missing => Ok(None),
            Self::Failed => Err(Offline),
            Self::Found(row) => Ok(Some(row.clone())),
        }
    }
}

// one struct serves both seams, so a scenario scripts each policy call and each lookup from the same case
pub(super) struct Scripted<'run> {
    journal: &'run Journal,
    outcomes: Mutex<VecDeque<ScriptOutcome>>,
    user: ScriptedLookup<LiteLLMUserTable>,
    project: ScriptedLookup<ProjectTable>,
    membership: ScriptedLookup<LiteLLMTeamMembership>,
}

impl<'run> Scripted<'run> {
    pub(super) fn new(
        journal: &'run Journal,
        outcomes: &[ScriptOutcome],
        user: ScriptedLookup<LiteLLMUserTable>,
        project: ScriptedLookup<ProjectTable>,
        membership: ScriptedLookup<LiteLLMTeamMembership>,
    ) -> Self {
        Self {
            journal,
            outcomes: Mutex::new(outcomes.iter().copied().collect()),
            user,
            project,
            membership,
        }
    }

    // an exhausted script allows, so an extra call still shows up in the journal
    fn verdict(&self) -> Result<(), PolicyFailure<BrokenCheck>> {
        let next = self
            .outcomes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(ScriptOutcome::Allow);
        match next {
            ScriptOutcome::Allow => Ok(()),
            ScriptOutcome::Deny => Err(PolicyFailure::Denied),
            ScriptOutcome::Broken => Err(PolicyFailure::Unexpected(BrokenCheck)),
        }
    }
}

// the user and project checks differ only in which allowlist they record, so one macro writes both
macro_rules! scripted_check {
    (
        $method:ident,
        ($($argument:ident: $argument_type:ty),* $(,)?),
        $record:expr $(,)?
    ) => {
        fn $method(
            &self,
            _router: Option<&dyn ModelAccessView>,
            $($argument: $argument_type,)*
        ) -> Result<(), PolicyFailure<BrokenCheck>> {
            self.journal.record($record);
            self.verdict()
        }
    };
}

impl SummaryModelChecks for Scripted<'_> {
    type Failure = BrokenCheck;

    fn object_call(
        &self,
        _router: Option<&dyn ModelAccessView>,
        call: &ObjectCall<'_>,
    ) -> Result<(), PolicyFailure<BrokenCheck>> {
        self.journal.record(CallRecord {
            object_type: Some(model_access_object_name(call.object_type)),
            model: Some(call.model.to_owned()),
            models: call.models.to_vec(),
            team_model_aliases: call.team_model_aliases.clone(),
            team_id: call.team_id.map(str::to_owned),
            ..CallRecord::new("object")
        });
        self.verdict()
    }

    scripted_check!(
        user_call,
        (model: &str, user: &LiteLLMUserTable),
        CallRecord {
            model: Some(model.to_owned()),
            models: user_models(user),
            ..CallRecord::new("user")
        },
    );

    scripted_check!(
        project_call,
        (model: &str, project: &ProjectTable),
        CallRecord {
            model: Some(model.to_owned()),
            models: project.models.clone(),
            ..CallRecord::new("project")
        },
    );
}

// the three lookups differ only in their row type and journal fields, so one macro writes all three
macro_rules! scripted_lookup {
    (
        $method:ident,
        ($($argument:ident: $argument_type:ty),* $(,)?),
        $row:ty,
        $lookup:ident,
        $record:expr $(,)?
    ) => {
        fn $method(
            &self,
            $($argument: $argument_type,)*
        ) -> impl Future<Output = Result<Option<$row>, Offline>> + Send {
            self.journal.record($record);
            ready(self.$lookup.answer())
        }
    };
}

impl SummaryAccessRepositories for Scripted<'_> {
    type Failure = Offline;

    scripted_lookup!(
        user,
        (user_id: &str, user_id_upsert: bool),
        LiteLLMUserTable,
        user,
        CallRecord {
            user_id: Some(user_id.to_owned()),
            user_id_upsert: Some(user_id_upsert),
            ..CallRecord::new("lookup_user")
        },
    );

    scripted_lookup!(
        project,
        (project_id: &str),
        ProjectTable,
        project,
        CallRecord {
            project_id: Some(project_id.to_owned()),
            ..CallRecord::new("lookup_project")
        },
    );

    scripted_lookup!(
        membership,
        (user_id: &str, team_id: &str),
        LiteLLMTeamMembership,
        membership,
        CallRecord {
            user_id: Some(user_id.to_owned()),
            team_id: Some(team_id.to_owned()),
            ..CallRecord::new("lookup_membership")
        },
    );
}
