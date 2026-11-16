pub mod cache;
pub mod error;
pub mod stored_skill;
mod tests;

use betterlite_auth::auth_identity::records::Principal;
use betterlite_context::dynamic_logging_cache::Clock;
use betterlite_db::managed_resource_rows::skills::SkillTable;

use crate::api::managed_resource_store::{
    get_resource_owner_scopes, is_proxy_admin, user_can_access_resource_owner,
};
use crate::skills::store::SkillPage;
use crate::skills::store::repository::SkillRepository;

use cache::{CachedSkill, SkillCacheLimits, SkillRowCache};
use error::StoredSkillReadError;
use stored_skill::StoredSkill;

pub struct StoredSkillReads<R> {
    repository: R,
    cache: SkillRowCache,
}

impl<R: SkillRepository> StoredSkillReads<R> {
    pub fn new(
        repository: R,
        limits: SkillCacheLimits,
        clock: Clock,
    ) -> Result<Self, StoredSkillReadError> {
        let cache = SkillRowCache::new(limits, clock)?;
        Ok(Self { repository, cache })
    }

    pub async fn list_skills(
        &self,
        page: SkillPage,
        principal: Option<&Principal>,
    ) -> Result<Vec<StoredSkill>, StoredSkillReadError> {
        self.repository.availability()?;
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
        rows.into_iter().map(StoredSkill::from_row).collect()
    }

    pub async fn get_skill(
        &self,
        skill_id: &str,
        principal: Option<&Principal>,
    ) -> Result<StoredSkill, StoredSkillReadError> {
        let Some(skill) = self.load_skill(skill_id).await? else {
            return Err(StoredSkillReadError::NotFound {
                skill_id: skill_id.to_owned(),
            });
        };
        if !user_can_access_resource_owner(skill.created_by.as_deref(), principal) {
            return Err(StoredSkillReadError::NotFound {
                skill_id: skill_id.to_owned(),
            });
        }
        StoredSkill::from_row(skill)
    }

    pub async fn load_skill(
        &self,
        skill_id: &str,
    ) -> Result<Option<SkillTable>, StoredSkillReadError> {
        match self.cache.lookup(skill_id)? {
            CachedSkill::Missing => return Ok(None),
            CachedSkill::Row(row) => return Ok(Some(*row)),
            CachedSkill::Miss => {}
        }
        self.repository.availability()?;
        let skill = self.repository.find_unique(skill_id).await?;
        self.cache.store(skill_id, skill.as_ref())?;
        Ok(skill)
    }
}
