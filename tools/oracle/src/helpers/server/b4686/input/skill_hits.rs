use betterlite_core::skills::search::SkillSearchResult;

use super::{FromFixture, NamedInput, SKILL_SCORE};

impl FromFixture<NamedInput> for SkillSearchResult {
    fn from_fixture(skill: &NamedInput) -> Self {
        Self {
            skill_id: skill.id.clone(),
            display_title: Some(skill.name.clone()),
            description: skill.description.clone(),
            score: SKILL_SCORE,
        }
    }
}
