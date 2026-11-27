use betterlite_core::skills::search::SkillSearchResult;

use super::{FromFixture, NamedInput, SKILL_SCORE};

impl FromFixture<NamedInput> for SkillSearchResult {
    fn from_fixture(input: &NamedInput) -> Self {
        Self {
            skill_id: input.id.clone(),
            display_title: Some(input.name.clone()),
            description: input.description.clone(),
            score: SKILL_SCORE,
        }
    }
}
