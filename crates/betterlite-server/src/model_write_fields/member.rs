use std::error::Error;

use async_trait::async_trait;
use betterlite_db::team_rows::team::LiteLLMTeamTable;

use crate::team_access::caller::TeamCaller;

// a create authorizes the new deployment; an update authorizes the patch against the stored deployment
pub enum MemberWrite<'write, Patch, Stored> {
    Create {
        deployment: &'write Stored,
    },
    Update {
        patch: &'write Patch,
        existing: &'write Stored,
    },
}

// B378's authorize_member_auto_router_write, passed in so an unregistered policy refuses instead of granting
#[async_trait]
pub trait MemberRouterWrite: Send + Sync {
    type Patch: Sync;
    type Stored: Sync;
    type Grant: Send;
    type Refusal: Error + Send + Sync + 'static;

    async fn authorize(
        &self,
        caller: &TeamCaller,
        team: &LiteLLMTeamTable,
        write: MemberWrite<'_, Self::Patch, Self::Stored>,
    ) -> Result<Self::Grant, Self::Refusal>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum ModelWriteGrant<Grant> {
    Authorized,
    MemberRouter(Grant),
}
