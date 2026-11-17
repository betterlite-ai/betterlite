use async_trait::async_trait;
use betterlite_core::api::stored_skill_reads::StoredSkillReads;
use betterlite_core::api::stored_skill_reads::error::StoredSkillReadError;
use betterlite_core::api::stored_skill_reads::stored_skill::StoredSkill;
use betterlite_core::skills::store::SkillPage;
use betterlite_core::skills::store::repository::SkillRepository;

// LiteLLM's MAX_INDEXED_SKILLS: the discovery index reads the newest page of at most 1000 skills
const MAX_INDEXED_SKILLS: i64 = 1_000;
const FIRST_PAGE_OFFSET: i64 = 0;

#[async_trait]
pub trait StoredSkills: Send + Sync {
    async fn list(&self) -> Result<Vec<StoredSkill>, StoredSkillReadError>;
    async fn get(&self, skill_id: &str) -> Result<Option<StoredSkill>, StoredSkillReadError>;
}

#[async_trait]
impl<R: SkillRepository> StoredSkills for StoredSkillReads<R> {
    async fn list(&self) -> Result<Vec<StoredSkill>, StoredSkillReadError> {
        self.list_skills(
            SkillPage {
                limit: MAX_INDEXED_SKILLS,
                offset: FIRST_PAGE_OFFSET,
            },
            None,
        )
        .await
    }

    // LiteLLM's stored_skill turns every ValueError of the get into None, which covers no row and no database
    async fn get(&self, skill_id: &str) -> Result<Option<StoredSkill>, StoredSkillReadError> {
        match self.get_skill(skill_id, None).await {
            Ok(skill) => Ok(Some(skill)),
            Err(StoredSkillReadError::NotFound { .. } | StoredSkillReadError::NoDatabase) => {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }
}
