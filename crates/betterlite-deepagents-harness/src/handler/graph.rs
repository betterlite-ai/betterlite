use std::collections::BTreeMap;
use std::io;
use std::pin::Pin;

use betterlite_harness::deepagents::sandbox_backend::ToolFilterMiddleware;
use betterlite_harness::deepagents::sandbox_backend::usage::UsageCallback;
use betterlite_harness::deepagents::transformation::INSTALL_HINT;
use betterlite_harness::deepagents::transformation::gateway::ChatModelKwargs;
use betterlite_harness::deepagents::transformation::message::Message;
use betterlite_harness::deepagents::transformation::tools::InterruptOnConfig;
use betterlite_harness::deepagents::transformation::update::{HitlDecision, Update};
use betterlite_harness::errors::HarnessError;
use betterlite_harness::types::JsonObject;
use futures_core::Stream;
use futures_util::stream;

use crate::sandbox_backend::Permissions;

pub type GraphStream = Pin<Box<dyn Stream<Item = Result<StreamPart, HarnessError>> + Send>>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraphInput {
    Messages(Message),
    Resume(BTreeMap<String, Vec<HitlDecision>>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StreamPart {
    Message { message: Message, node: String },
    Update(Update),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GraphState {
    pub messages: Vec<Message>,
    pub structured_response: Option<JsonObject>,
}

#[derive(Clone)]
pub struct RunConfig {
    pub thread_id: String,
    pub recursion_limit: u32,
    pub usage_callback: Option<UsageCallback>,
}

#[derive(Clone, Debug)]
pub struct AgentSpec {
    pub chat_model: ChatModelKwargs,
    pub system_prompt: Option<String>,
    pub middleware: Option<ToolFilterMiddleware>,
    pub subagents: Vec<JsonObject>,
    pub skills: Option<Vec<String>>,
    pub permissions: Permissions,
    pub interrupt_on: Option<BTreeMap<String, InterruptOnConfig>>,
    pub output_schema: Option<JsonObject>,
}

pub trait Graph: Send + Sync {
    fn stream(&self, input: GraphInput, config: RunConfig) -> GraphStream;
    fn state(&self, config: RunConfig) -> GraphState;
}

pub trait AgentRuntime: Send + Sync {
    type Graph: Graph;

    // fails when the runtime's own dependencies are missing, before any spec work, as LiteLLM's load_deps raises
    fn load_deps(&self) -> Result<(), HarnessError>;
    fn build_agent(&self, spec: AgentSpec) -> Result<Self::Graph, HarnessError>;
    fn general_subagent(&self) -> JsonObject;
    fn openai_messages(&self, messages: &[Message]) -> Vec<JsonObject>;
    // the runtime owns the model calls and their prices, so it builds the callback; None means it has no pricer
    fn usage_callback(&self, cost_model: Option<String>) -> Option<UsageCallback>;
}

pub struct MissingRuntime;

pub struct MissingGraph;

impl AgentRuntime for MissingRuntime {
    type Graph = MissingGraph;

    fn load_deps(&self) -> Result<(), HarnessError> {
        Err(HarnessError::InstallFailed(io::Error::new(
            io::ErrorKind::NotFound,
            INSTALL_HINT,
        )))
    }

    fn build_agent(&self, _spec: AgentSpec) -> Result<MissingGraph, HarnessError> {
        self.load_deps()?;
        Ok(MissingGraph)
    }

    fn general_subagent(&self) -> JsonObject {
        JsonObject::new()
    }

    fn openai_messages(&self, _messages: &[Message]) -> Vec<JsonObject> {
        Vec::new()
    }

    fn usage_callback(&self, _cost_model: Option<String>) -> Option<UsageCallback> {
        None
    }
}

impl Graph for MissingGraph {
    fn stream(&self, _input: GraphInput, _config: RunConfig) -> GraphStream {
        Box::pin(stream::empty())
    }

    fn state(&self, _config: RunConfig) -> GraphState {
        GraphState::default()
    }
}
