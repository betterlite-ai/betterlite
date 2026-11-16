use std::sync::Arc;

use betterlite_context::dynamic_logging_cache::Clock;
use betterlite_core::skills::search::{
    DEFAULT_SKILL_SEARCH_TOP_K, HostedSkillSearchRequest, SkillSearchEmbedding, SkillSearchIndex,
    SkillSearchRuntime, search_hosted_skills, search_skills, skill_search_text,
};
use betterlite_core::skills::store::{HostedSkillStore, SkillIdSource};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_semantic::Embedder;
use serde_json::json;

use super::input::Input;
use super::scripted::{CACHE_CLOCK, Effects, ScriptedEmbedder, ScriptedRepository, TEST_SKILL_ID};
use super::values::{
    hosted_error, http_error, invalid_input, outcome_value, principal_of, skill_row, skill_rows,
    store_error,
};

fn embedding_model(input: &Input) -> Result<&str, Box<HelperError>> {
    input
        .embedding_model
        .as_deref()
        .ok_or_else(|| invalid_input("embedding_model is required".to_owned()))
}

pub(super) fn text_value(input: &Input) -> HelperResult {
    let Some(fields) = input.skill.as_ref() else {
        return Err(invalid_input("skill is required".to_owned()));
    };
    let skill = skill_row(fields)?;
    Ok(json!({"text": skill_search_text(&skill)}))
}

pub(super) async fn index_value(input: &Input, effects: &Effects) -> HelperResult {
    let embedder = ScriptedEmbedder::new(input, effects)?;
    let index = SkillSearchIndex::default();
    let outcome = index
        .search(
            &input.query,
            &skill_rows(&input.skills)?,
            input.top_k.unwrap_or(DEFAULT_SKILL_SEARCH_TOP_K),
            SkillSearchEmbedding {
                embedder: &embedder,
                embedding_model: embedding_model(input)?,
            },
        )
        .await
        .map_err(|error| http_error(&error))?;
    outcome_value(&outcome, effects.take())
}

pub(super) async fn search_value(input: &Input, effects: &Effects) -> HelperResult {
    let embedder = ScriptedEmbedder::new(input, effects)?;
    let router: Option<&dyn Embedder> = input.router.then_some(&embedder);
    let index = SkillSearchIndex::default();
    let outcome = search_skills(
        &input.query,
        &skill_rows(&input.skills)?,
        input.top_k.unwrap_or(DEFAULT_SKILL_SEARCH_TOP_K),
        SkillSearchRuntime {
            embedder: router,
            embedding_model: input.embedding_model.as_deref(),
            index: &index,
        },
    )
    .await
    .map_err(|error| http_error(&error))?;
    outcome_value(&outcome, effects.take())
}

pub(super) async fn hosted_value(input: &Input, effects: &Effects) -> HelperResult {
    let embedder = ScriptedEmbedder::new(input, effects)?;
    let router: Option<&dyn Embedder> = input.router.then_some(&embedder);
    let index = SkillSearchIndex::default();
    let repository = ScriptedRepository::new(input, effects)?;
    let ids: SkillIdSource = Arc::new(|| TEST_SKILL_ID.to_owned());
    let clock: Clock = Arc::new(|| CACHE_CLOCK);
    let store =
        HostedSkillStore::new(repository, ids, clock).map_err(|error| store_error(&error))?;
    let principal = input.principal.as_ref().map(principal_of).transpose()?;
    let outcome = search_hosted_skills(
        &store,
        principal.as_ref(),
        HostedSkillSearchRequest {
            custom_llm_provider: input.custom_llm_provider.as_deref(),
            query: &input.query,
            top_k: input.top_k.unwrap_or(DEFAULT_SKILL_SEARCH_TOP_K),
            runtime: SkillSearchRuntime {
                embedder: router,
                embedding_model: input.embedding_model.as_deref(),
                index: &index,
            },
        },
    )
    .await
    .map_err(|error| hosted_error(&error))?;
    outcome_value(&outcome, effects.take())
}
