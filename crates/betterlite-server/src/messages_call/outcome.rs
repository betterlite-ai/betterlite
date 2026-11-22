use async_trait::async_trait;
use betterlite_openai::openai_base_config::Object as JsonMap;

use crate::keepalive_interval::DeploymentLookup;
use crate::shared_stream::call::SharedSource;

use super::MessagesCallError;

pub enum MessagesOutcome {
    Message {
        body: JsonMap,
        hidden: JsonMap,
    },
    Stream {
        source: SharedSource,
        hidden: JsonMap,
    },
}

impl MessagesOutcome {
    // LiteLLM's _hidden_params, which every route shape carries
    #[must_use]
    pub const fn hidden_mut(&mut self) -> &mut JsonMap {
        match self {
            Self::Message { hidden, .. } | Self::Stream { hidden, .. } => hidden,
        }
    }
}

// litellm.anthropic_messages(**data): the provider call behind the direct branch
#[async_trait]
pub trait MessagesDispatch: Send + Sync {
    async fn dispatch(&self, data: JsonMap) -> Result<MessagesOutcome, MessagesCallError>;
}

// The router facts route_llm_request reads for the messages route
#[async_trait]
pub trait MessagesRouting: DeploymentLookup + Send + Sync {
    fn all_blocked(&self, model: &str, team_id: Option<&str>) -> bool;

    fn map_team_model(&self, model: Option<&str>, team_id: &str) -> Option<String>;

    fn is_recognized_model(&self, model: &str) -> bool;

    fn model_names_has(&self, model: &str) -> bool;

    fn team_public_model_names_has(&self, model: &str) -> bool;

    fn pass_through_all_models(&self) -> bool;

    fn has_default_or_pattern(&self) -> bool;

    fn is_deployment_name(&self, model: &str) -> bool;

    async fn anthropic_messages(
        &self,
        data: JsonMap,
        specific_deployment: bool,
    ) -> Result<MessagesOutcome, MessagesCallError>;

    async fn a2a_agent(&self, data: JsonMap) -> Option<Result<MessagesOutcome, MessagesCallError>>;
}
