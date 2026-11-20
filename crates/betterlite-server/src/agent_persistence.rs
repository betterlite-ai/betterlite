macro_rules! push_columns {
    ($list:expr, $values:expr, $($name:literal => $field:ident : $cast:literal),* $(,)?) => {
        $(push_present($list, $name, $values.$field.as_ref(), $cast);)*
    };
}

pub mod change_publisher;
pub mod create;
pub mod error;
pub mod identity_write;
pub mod json_text;
pub mod patch;
pub mod permission_write;
pub mod remove;
pub mod replace;
pub mod row;
pub mod write_plan;

#[path = "agent_persistence/tests/support.rs"]
mod support;

use std::future::Future;

use betterlite_core::agents::AgentConfig;
use betterlite_core::agents::AgentResponse;
use betterlite_core::agents::PatchAgentRequest;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_redact::structured::SensitiveDataMasker;
use chrono::Utc;

use self::create::store_agent;
use self::error::AgentPersistenceError;
use self::error::open_failure;
use self::patch::UpdateRequest;
use self::patch::commit_update;
use self::patch::existing_agent;
use self::remove::delete_agent;
use self::replace::patch_request;
use self::write_plan::ColumnSource;
use self::write_plan::ConfigMode;
use self::write_plan::ConfigSource;
use self::write_plan::PatchSource;
use self::write_plan::PlanContext;
use self::write_plan::plan_write;
use crate::agent_rows::codec::StoredAgent;

pub trait AgentChangePublisher: Send + Sync {
    fn agents_changed(&self) -> impl Future<Output = ()> + Send;
}

pub struct AgentStore<P: AgentChangePublisher> {
    pub store: PostgresStore,
    pub masker: SensitiveDataMasker,
    pub max_json_depth: usize,
    pub publisher: P,
}

impl<P: AgentChangePublisher> AgentStore<P> {
    fn context(&self) -> PlanContext<'_> {
        PlanContext {
            masker: &self.masker,
            max_json_depth: self.max_json_depth,
            now: Utc::now(),
        }
    }

    async fn commit(
        &self,
        agent_id: &str,
        source: &dyn ColumnSource,
        existing: Option<&StoredAgent>,
        actor: &str,
    ) -> Result<AgentResponse, AgentPersistenceError> {
        let context = self.context();
        let plan = plan_write(source, existing, actor, &context)?;
        let request = UpdateRequest {
            agent_id,
            plan: &plan,
            permission: source.permission(),
            existing_permission: existing.and_then(|record| record.object_permission_id.as_deref()),
            missing: source.not_found(agent_id),
        };
        commit_update(self, &request).await
    }

    pub async fn add_agent_to_db(
        &self,
        agent: &AgentConfig,
        created_by: &str,
        agent_id: Option<&str>,
    ) -> Result<AgentResponse, AgentPersistenceError> {
        let context = self.context();
        let source = ConfigSource {
            agent,
            mode: ConfigMode::Create,
        };
        let plan = plan_write(&source, None, created_by, &context)?;
        store_agent(self, agent, &plan, agent_id).await
    }

    pub async fn update_agent_in_db(
        &self,
        agent_id: &str,
        agent: &AgentConfig,
        updated_by: &str,
    ) -> Result<AgentResponse, AgentPersistenceError> {
        if agent.agent_card_params.is_some() {
            let existing = existing_agent(&self.store, agent_id).await?;
            self.commit(
                agent_id,
                &ConfigSource {
                    agent,
                    mode: ConfigMode::Replace,
                },
                existing.as_ref(),
                updated_by,
            )
            .await
        } else {
            let replacement = patch_request(agent);
            self.patch_agent_in_db(agent_id, &replacement, updated_by)
                .await
        }
    }

    pub async fn patch_agent_in_db(
        &self,
        agent_id: &str,
        agent: &PatchAgentRequest,
        updated_by: &str,
    ) -> Result<AgentResponse, AgentPersistenceError> {
        let source = PatchSource { agent };
        let missing = source.not_found(agent_id);
        let existing = existing_agent(&self.store, agent_id).await?;
        let Some(stored) = existing else {
            return Err(AgentPersistenceError::NotFound(missing));
        };
        self.commit(agent_id, &source, Some(&stored), updated_by)
            .await
    }

    pub async fn delete_agent_from_db(&self, agent_id: &str) -> Result<(), AgentPersistenceError> {
        let missing = format!("Agent not found, passed agent_id={agent_id}");
        let mut client = self.store.session().await.map_err(open_failure)?;
        delete_agent(&mut client, agent_id, &missing).await
    }
}
