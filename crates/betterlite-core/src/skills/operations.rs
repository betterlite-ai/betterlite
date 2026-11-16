mod files;
mod logging;
mod metadata;
mod projection;
mod tests;

use betterlite_anthropic::skills::wire::{DeleteSkillResponse, ListSkillsResponse, Skill};
use betterlite_auth::auth_identity::records::Principal;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use thiserror::Error;

use crate::skills::store::repository::SkillRepository;
use crate::skills::store::{HostedSkillStore, NewSkill, SkillPage, StoreError};

use files::selected_file;
use logging::{SkillLog, log_skill_call};
use projection::db_skill_to_response;

#[derive(Debug, Error)]
pub enum HostedSkillOpsError {
    #[error(transparent)]
    Context(#[from] LoggingStateError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("the skill timestamp could not be rendered as an ISO datetime")]
    Timestamp(#[source] serde_json::Error),
}

// LiteLLM's transformation handler takes the caller's arguments and its kwargs, which stay empty for skills
#[derive(Default)]
pub struct HostedSkillCreation<'request> {
    pub display_title: Option<&'request str>,
    pub description: Option<&'request str>,
    pub instructions: Option<&'request str>,
    pub files: Option<&'request [SkillUpload<'request>]>,
    pub file_content: Option<&'request [u8]>,
    pub file_name: Option<&'request str>,
    pub file_type: Option<&'request str>,
    pub metadata: Option<&'request IndexMap<String, Json>>,
    pub user_id: Option<&'request str>,
    pub principal: Option<&'request Principal>,
    pub litellm_call_id: Option<&'request str>,
}

// The single-skill call get and delete share: the row's id, the caller scope, the call id, and the logging state
pub struct SkillTarget<'request> {
    pub logging: &'request mut LoggingState,
    pub id: &'request str,
    pub principal: Option<&'request Principal>,
    pub call_id: Option<&'request str>,
}

// One uploaded tuple; a two-element tuple names no MIME type and falls back to application/zip
pub struct SkillUpload<'request> {
    pub file_name: &'request str,
    pub content: &'request [u8],
    pub file_type: SkillUploadType<'request>,
}

#[derive(Clone, Copy)]
pub enum SkillUploadType<'request> {
    Defaulted,
    Given(Option<&'request str>),
}

// LiteLLM's LiteLLMSkillsTransformationHandler over the injected store and logging context
pub struct HostedSkillOps<R> {
    pub store: HostedSkillStore<R>,
}

impl<R: SkillRepository> HostedSkillOps<R> {
    pub async fn create_skill(
        &self,
        logging: &mut LoggingState,
        request: HostedSkillCreation<'_>,
    ) -> Result<Skill, HostedSkillOpsError> {
        let chosen = selected_file(
            request.files,
            request.file_content,
            request.file_name,
            request.file_type,
        );
        let data = NewSkill {
            display_title: request.display_title.map(str::to_owned),
            description: request.description.map(str::to_owned),
            instructions: request.instructions.map(str::to_owned),
            metadata: request.metadata.map(metadata::encode),
            file_content: chosen.content.map(<[u8]>::to_vec),
            file_name: chosen.name.map(str::to_owned),
            file_type: chosen.file_type.map(str::to_owned),
        };
        log_skill_call(
            logging,
            SkillLog::Create(request.display_title),
            request.litellm_call_id,
        )?;
        let row = self
            .store
            .create_skill(&data, request.user_id, request.principal)
            .await?;
        db_skill_to_response(&row)
    }

    pub async fn list_skills(
        &self,
        logging: &mut LoggingState,
        page: SkillPage,
        principal: Option<&Principal>,
        litellm_call_id: Option<&str>,
    ) -> Result<ListSkillsResponse, HostedSkillOpsError> {
        log_skill_call(logging, SkillLog::List(page), litellm_call_id)?;
        let rows = self.store.list_skills(page, principal).await?;
        let data = rows
            .iter()
            .map(db_skill_to_response)
            .collect::<Result<Vec<_>, _>>()?;
        // LiteLLM reports a full page as more, even for a zero limit, and never asks for the next row
        let returned = i64::try_from(data.len()).unwrap_or(i64::MAX);
        Ok(ListSkillsResponse {
            has_more: returned >= page.limit,
            data,
            next_page: None,
        })
    }

    pub async fn get_skill(&self, target: SkillTarget<'_>) -> Result<Skill, HostedSkillOpsError> {
        log_skill_call(target.logging, SkillLog::Skill(target.id), target.call_id)?;
        db_skill_to_response(&self.store.get_skill(target.id, target.principal).await?)
    }

    pub async fn delete_skill(
        &self,
        target: SkillTarget<'_>,
    ) -> Result<DeleteSkillResponse, HostedSkillOpsError> {
        log_skill_call(target.logging, SkillLog::Skill(target.id), target.call_id)?;
        Ok(self.store.delete_skill(target.id, target.principal).await?)
    }
}
