mod tests;

use betterlite_auth::auth_identity::records::Principal;
use betterlite_context::dynamic_logging_cache::Clock;
use betterlite_semantic::Embedder;

use crate::skills::operations::HostedSkillOps;
use crate::skills::sandbox::executor::SandboxExecutor;
use crate::skills::sandbox::session::SessionOpener;
use crate::skills::search::{
    HostedSkillSearchError, HostedSkillSearchRequest, SkillSearchIndex, SkillSearchOutcome,
    SkillSearchRuntime, search_hosted_skills,
};
use crate::skills::store::repository::SkillRepository;
use crate::skills::store::{HostedSkillStore, SkillIdSource, StoreError};

// The hosted skill services B1502-B1506 built; B1504's prompt and tool functions are free functions, so no service to hold here.
pub struct HostedSkills<R, S> {
    pub ops: HostedSkillOps<R>,
    pub search: SkillSearchIndex,
    pub sandbox: SandboxExecutor<S>,
}

impl<R: SkillRepository, S: SessionOpener> HostedSkills<R, S> {
    // The caller builds the sandbox leaf with B1506's opener, image and timeout; this module wires the store once.
    pub fn new(
        repository: R,
        ids: SkillIdSource,
        clock: Clock,
        sandbox: SandboxExecutor<S>,
    ) -> Result<Self, StoreError> {
        let store = HostedSkillStore::new(repository, ids, clock)?;
        Ok(Self {
            ops: HostedSkillOps { store },
            search: SkillSearchIndex::default(),
            sandbox,
        })
    }

    // B1505's search reads the same store B1503's operations write, so a created skill is searchable at once.
    pub async fn search(
        &self,
        principal: Option<&Principal>,
        search: HostedSkillSearch<'_>,
    ) -> Result<SkillSearchOutcome, HostedSkillSearchError> {
        search_hosted_skills(
            &self.ops.store,
            principal,
            HostedSkillSearchRequest {
                custom_llm_provider: search.custom_llm_provider,
                query: search.query,
                top_k: search.top_k,
                runtime: SkillSearchRuntime {
                    embedder: search.embedder,
                    embedding_model: search.embedding_model,
                    index: &self.search,
                },
            },
        )
        .await
    }
}

// What a caller supplies to a search; the index itself comes from the composition.
#[derive(Clone, Copy)]
pub struct HostedSkillSearch<'search> {
    pub custom_llm_provider: Option<&'search str>,
    pub query: &'search str,
    pub top_k: usize,
    pub embedder: Option<&'search dyn Embedder>,
    pub embedding_model: Option<&'search str>,
}
