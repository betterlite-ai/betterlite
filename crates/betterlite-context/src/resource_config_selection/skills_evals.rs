use betterlite_models::identity::provider_id::ProviderId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillsConfigTag {
    Anthropic,
}

#[must_use]
pub fn select_skills_config(provider: ProviderId) -> Option<SkillsConfigTag> {
    if provider != ProviderId::Anthropic {
        return None;
    }
    Some(SkillsConfigTag::Anthropic)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvalsConfigTag {
    OpenAI,
}

#[must_use]
pub fn select_evals_config(provider: ProviderId) -> Option<EvalsConfigTag> {
    if provider != ProviderId::Openai {
        return None;
    }
    Some(EvalsConfigTag::OpenAI)
}
