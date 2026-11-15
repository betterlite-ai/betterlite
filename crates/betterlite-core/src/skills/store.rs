mod cache;
pub mod repository;
mod tests;

use std::collections::BTreeMap;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_anthropic::skills::wire::DeleteSkillResponse;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_context::dynamic_logging_cache::Clock;
use betterlite_context::local_ttl_cache::CacheError;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::managed_resource_rows::skills::SkillTable;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_logging::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};
use thiserror::Error;

use crate::api::managed_resource_store::{
    get_primary_owner_scope, get_resource_owner_scopes, is_proxy_admin,
    user_can_access_resource_owner,
};

use cache::{CachedRow, SkillCache};
use repository::{SkillRepository, SkillWrite};

pub const SKILL_ID_PREFIX: &str = "litellm_skill_";
pub const MAX_SKILLS_PER_SEARCH: i64 = 5000;
pub const DEFAULT_PAGE_LIMIT: i64 = 20;
pub const DEFAULT_PAGE_OFFSET: i64 = 0;
pub const NEGATIVE_SKILL_SENTINEL: &str = "__litellm_skill_not_found__";
const DELETED_SKILL_TYPE: &str = "skill_deleted";
const CUSTOM_SOURCE: &str = "custom";

// A uuid source, so a test or the wiring names the identifier the handler would have drawn
pub type SkillIdSource = Arc<dyn Fn() -> String + Send + Sync>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NewSkill {
    pub display_title: Option<String>,
    pub description: Option<String>,
    pub instructions: Option<String>,
    pub metadata: Option<BTreeMap<String, JsonData>>,
    pub file_content: Option<Vec<u8>>,
    pub file_name: Option<String>,
    pub file_type: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SkillPage {
    pub limit: i64,
    pub offset: i64,
}

impl Default for SkillPage {
    fn default() -> Self {
        Self {
            limit: DEFAULT_PAGE_LIMIT,
            offset: DEFAULT_PAGE_OFFSET,
        }
    }
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("Prisma client is not initialized. Database connection required for LiteLLM skills.")]
    NoDatabase,
    #[error("Unable to record skill ownership: caller has no identity scope.")]
    MissingIdentity,
    #[error("Skill not found: {skill_id}")]
    NotFound { skill_id: String },
    #[error("the skill repository failed")]
    Repository {
        #[source]
        source: Box<TableError>,
    },
    #[error("the skill metadata could not be encoded as JSON")]
    Metadata(#[source] PrivateJsonError),
    #[error(transparent)]
    Cache(#[from] CacheError),
}

impl StoreError {
    // LiteLLM raises ValueError for exactly these three, and the hook swallows them without a warning
    #[must_use]
    pub const fn is_value_error(&self) -> bool {
        matches!(
            self,
            Self::NoDatabase | Self::MissingIdentity | Self::NotFound { .. }
        )
    }
}

impl From<TableError> for StoreError {
    fn from(error: TableError) -> Self {
        match error {
            TableError::Database(DbError::NoDbConnected) => Self::NoDatabase,
            other @ (TableError::Database(_)
            | TableError::Batch(_)
            | TableError::Row(_)
            | TableError::MissingRow
            | TableError::RowCount { .. }
            | TableError::Publish { .. }) => Self::Repository {
                source: Box::new(other),
            },
        }
    }
}

pub struct HostedSkillStore<R> {
    repository: R,
    ids: SkillIdSource,
    cache: SkillCache,
}

impl<R: SkillRepository> HostedSkillStore<R> {
    pub fn new(repository: R, ids: SkillIdSource, clock: Clock) -> Result<Self, StoreError> {
        let cache = SkillCache::new(clock)?;
        Ok(Self {
            repository,
            ids,
            cache,
        })
    }

    pub async fn create_skill(
        &self,
        data: &NewSkill,
        user_id: Option<&str>,
        principal: Option<&Principal>,
    ) -> Result<SkillTable, StoreError> {
        self.repository.availability()?;
        let skill_id = format!("{SKILL_ID_PREFIX}{}", (self.ids)());
        let chosen_owner = get_primary_owner_scope(principal)
            .map(|scope| scope.0)
            .or_else(|| user_id.map(str::to_owned));
        let Some(owner) = chosen_owner else {
            return Err(StoreError::MissingIdentity);
        };
        let metadata = data
            .metadata
            .as_ref()
            .map(|metadata| safe_dumps(metadata, DEFAULT_MAX_RECURSE_DEPTH, None))
            .transpose()
            .map_err(StoreError::Metadata)?;
        let write = SkillWrite {
            skill_id: skill_id.clone(),
            display_title: data.display_title.clone(),
            description: data.description.clone(),
            instructions: data.instructions.clone(),
            source: CUSTOM_SOURCE.to_owned(),
            created_by: owner.clone(),
            updated_by: owner,
            metadata,
            file_content: data
                .file_content
                .as_deref()
                .map(|bytes| STANDARD.encode(bytes)),
            file_name: data.file_name.clone(),
            file_type: data.file_type.clone(),
        };
        tracing::debug!(skill_id = %skill_id, "LiteLLMSkillsHandler: Creating skill");
        Ok(self.repository.create(write).await?)
    }

    pub async fn list_skills(
        &self,
        page: SkillPage,
        principal: Option<&Principal>,
    ) -> Result<Vec<SkillTable>, StoreError> {
        self.repository.availability()?;
        tracing::debug!(
            limit = page.limit,
            offset = page.offset,
            "LiteLLMSkillsHandler: Listing skills"
        );
        let scopes = (principal.is_some() && !is_proxy_admin(principal))
            .then(|| get_resource_owner_scopes(principal));
        if scopes.as_ref().is_some_and(Vec::is_empty) {
            return Ok(Vec::new());
        }
        let created_by = scopes.map(|allowed| {
            allowed
                .into_iter()
                .map(|scope| scope.0)
                .collect::<Vec<String>>()
        });
        let rows = self.repository.find_many(page, created_by).await?;
        Ok(rows)
    }

    pub async fn list_skills_for_search(
        &self,
        principal: Option<&Principal>,
    ) -> Result<Vec<SkillTable>, StoreError> {
        self.list_skills(
            SkillPage {
                limit: MAX_SKILLS_PER_SEARCH,
                offset: DEFAULT_PAGE_OFFSET,
            },
            principal,
        )
        .await
    }

    pub async fn get_skill(
        &self,
        skill_id: &str,
        principal: Option<&Principal>,
    ) -> Result<SkillTable, StoreError> {
        tracing::debug!(skill_id = %skill_id, "LiteLLMSkillsHandler: Getting skill");
        self.authorized_skill(skill_id, principal).await
    }

    pub async fn delete_skill(
        &self,
        skill_id: &str,
        principal: Option<&Principal>,
    ) -> Result<DeleteSkillResponse, StoreError> {
        self.repository.availability()?;
        tracing::debug!(skill_id = %skill_id, "LiteLLMSkillsHandler: Deleting skill");
        self.authorized_skill(skill_id, principal).await?;
        self.repository.delete(skill_id).await?;
        self.cache.store(skill_id, None)?;
        Ok(DeleteSkillResponse {
            id: skill_id.to_owned(),
            r#type: DELETED_SKILL_TYPE.to_owned(),
        })
    }

    pub async fn fetch_skill_from_db(
        &self,
        skill_id: &str,
        principal: Option<&Principal>,
    ) -> Option<SkillTable> {
        match self.get_skill(skill_id, principal).await {
            Ok(skill) => Some(skill),
            Err(error) if error.is_value_error() => None,
            Err(error) => {
                tracing::warn!(
                    skill_id = %skill_id,
                    error = %error,
                    "LiteLLMSkillsHandler: Error fetching skill"
                );
                None
            }
        }
    }

    // The cache answers first, so a caller can reach a row while the database is down
    async fn authorized_skill(
        &self,
        skill_id: &str,
        principal: Option<&Principal>,
    ) -> Result<SkillTable, StoreError> {
        let Some(skill) = self.load_skill(skill_id).await? else {
            return Err(StoreError::NotFound {
                skill_id: skill_id.to_owned(),
            });
        };
        if !user_can_access_resource_owner(skill.created_by.as_deref(), principal) {
            return Err(StoreError::NotFound {
                skill_id: skill_id.to_owned(),
            });
        }
        Ok(skill)
    }

    async fn load_skill(&self, skill_id: &str) -> Result<Option<SkillTable>, StoreError> {
        match self.cache.lookup(skill_id)? {
            CachedRow::Missing => return Ok(None),
            CachedRow::Row(row) => return Ok(Some(*row)),
            CachedRow::Miss => {}
        }
        self.repository.availability()?;
        let skill = self.repository.find_unique(skill_id).await?;
        self.cache.store(skill_id, skill.as_ref())?;
        Ok(skill)
    }
}
