use std::future::{Future, ready};

use betterlite_anthropic::skills::wire::{DeleteSkillResponse, ListSkillsResponse, Skill};
use betterlite_core::api::skills::create::hosted::{HostedCreateSkillRequest, HostedSkillsCreator};
use betterlite_core::api::skills::get_delete::handler::SkillsHandlerError;
use betterlite_core::api::skills::get_delete::hosted::{HostedSkillRequest, HostedSkillsHandler};
use betterlite_core::api::skills::list::hosted::{HostedListSkillsRequest, HostedSkillsLister};

use crate::skill_calls::error::NO_DATABASE;

// The litellm_proxy path reads a database this package does not have; the session answers it before the service asks.
pub(crate) struct NoDatabaseSkills;

impl NoDatabaseSkills {
    fn failure() -> SkillsHandlerError {
        SkillsHandlerError::Provider {
            status_code: 500,
            message: NO_DATABASE.to_owned(),
            status_code_is_synthesized: true,
            body_code: None,
            body_message: None,
        }
    }
}

impl HostedSkillsHandler for NoDatabaseSkills {
    fn get_skill(
        &self,
        _request: HostedSkillRequest<'_>,
    ) -> impl Future<Output = Result<Skill, SkillsHandlerError>> + Send {
        ready(Err(Self::failure()))
    }

    fn delete_skill(
        &self,
        _request: HostedSkillRequest<'_>,
    ) -> impl Future<Output = Result<DeleteSkillResponse, SkillsHandlerError>> + Send {
        ready(Err(Self::failure()))
    }
}

impl HostedSkillsCreator for NoDatabaseSkills {
    fn create_skill(
        &self,
        _request: HostedCreateSkillRequest<'_>,
    ) -> impl Future<Output = Result<Skill, SkillsHandlerError>> + Send {
        ready(Err(Self::failure()))
    }
}

impl HostedSkillsLister for NoDatabaseSkills {
    fn list_skills(
        &self,
        _request: HostedListSkillsRequest<'_>,
    ) -> impl Future<Output = Result<ListSkillsResponse, SkillsHandlerError>> + Send {
        ready(Err(Self::failure()))
    }
}
