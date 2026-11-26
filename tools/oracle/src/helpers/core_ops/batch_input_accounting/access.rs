use std::convert::Infallible;

use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_core::api::batch_input_accounting::error::BatchHttpError;
use betterlite_core::api::batch_input_accounting::ports::{
    BatchAccessCheck, BatchKeyGrant, BatchModelAccess, BatchTeamLookup,
};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::input::{AccessScript, Lookup, Outcome};

type Checked = Result<BatchAccessCheck, Infallible>;
type Granted = Result<BatchKeyGrant, Infallible>;

// The team the fake lookup returns: the models the team is allowed to call
pub(super) struct TeamModels(pub(super) Vec<String>);

// The scripted auth slice: a team row, the key's own check and the denial text every refusal carries
pub(super) struct ScriptedAccess {
    pub(super) script: AccessScript,
}

pub(super) fn http_denial() -> BatchHttpError {
    let mut detail = JsonObject::new();
    detail.insert("error".to_owned(), Json::Text("passed through".to_owned()));
    BatchHttpError {
        status: 401,
        detail: Json::Object(detail),
    }
}

impl ScriptedAccess {
    fn listed(&self, model: &str, team: &TeamModels) -> BatchAccessCheck {
        if team.0.is_empty() || team.0.iter().any(|name| name == "*" || name == model) {
            BatchAccessCheck::Allowed
        } else {
            BatchAccessCheck::Denied {
                message: self.script.message.clone(),
            }
        }
    }

    fn key_check(&self) -> BatchAccessCheck {
        match self.script.key {
            Outcome::Allow => BatchAccessCheck::Allowed,
            Outcome::Deny => BatchAccessCheck::Denied {
                message: self.script.message.clone(),
            },
            Outcome::Fail => BatchAccessCheck::Failed {
                message: self.script.message.clone(),
            },
            Outcome::Http => BatchAccessCheck::Http(http_denial()),
        }
    }
}

#[async_trait]
impl BatchModelAccess for ScriptedAccess {
    type Team = TeamModels;
    type Error = Infallible;

    fn database_available(&self) -> bool {
        self.script.database
    }

    async fn team(&self, _team_id: &str) -> Result<BatchTeamLookup<TeamModels>, Infallible> {
        Ok(match self.script.team {
            Lookup::Loaded => BatchTeamLookup::Loaded(TeamModels(self.script.team_models.clone())),
            Lookup::Failed => BatchTeamLookup::Failed,
            Lookup::Http => BatchTeamLookup::Http(http_denial()),
        })
    }

    async fn team_can_access(&self, model: &str, _: &UserAPIKeyAuth, team: &TeamModels) -> Checked {
        Ok(self.listed(model, team))
    }

    async fn key_group_grants(&self, _: &str, _: &UserAPIKeyAuth, _: &TeamModels) -> Granted {
        Ok(BatchKeyGrant::Denied)
    }

    async fn member_can_access(&self, _: &str, _: &UserAPIKeyAuth, _: &TeamModels) -> Checked {
        Ok(BatchAccessCheck::Allowed)
    }

    async fn key_can_call(&self, _: &str, _: &UserAPIKeyAuth) -> Checked {
        Ok(self.key_check())
    }
}
