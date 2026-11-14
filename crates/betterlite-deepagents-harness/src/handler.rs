pub mod graph;

mod build;
mod tests;
mod turn;

use std::collections::BTreeSet;
use std::future::{Future, ready};

use futures_core::Stream;

use betterlite_harness::context::SessionContext;
use betterlite_harness::deepagents::sandbox_backend::usage::UsageCallback;
use betterlite_harness::deepagents::transformation::config::recursion_limit;
use betterlite_harness::errors::HarnessError;
use betterlite_harness::events::Event;
use betterlite_harness::handlers::BaseHarnessHandler;
use betterlite_harness::sandbox::Sandbox;
use betterlite_harness::transformation::BaseHarnessConfig;
use betterlite_harness::types::JsonObject;

use graph::{AgentRuntime, RunConfig};

pub struct DeepAgentsHandler<C, R: AgentRuntime> {
    pub config: C,
    pub runtime: R,
    pub graph: Option<R::Graph>,
    pub thread_id: Option<String>,
    pub skip_tools: BTreeSet<String>,
}

impl<C, R> DeepAgentsHandler<C, R>
where
    C: BaseHarnessConfig,
    R: AgentRuntime,
{
    fn run_config<S>(
        &self,
        context: &SessionContext<S>,
        usage_callback: Option<UsageCallback>,
    ) -> RunConfig {
        RunConfig {
            thread_id: self
                .thread_id
                .clone()
                .unwrap_or_else(|| context.session_id.clone()),
            recursion_limit: recursion_limit(context),
            usage_callback,
        }
    }

    fn history_messages<S>(
        &self,
        context: &SessionContext<S>,
    ) -> Result<Vec<JsonObject>, HarnessError> {
        let graph = self.graph.as_ref().ok_or(HarnessError::SessionClosed)?;
        let state = graph.state(self.run_config(context, None));
        Ok(self.runtime.openai_messages(&state.messages))
    }
}

impl<C, R> BaseHarnessHandler for DeepAgentsHandler<C, R>
where
    C: BaseHarnessConfig + Send + Sync,
    R: AgentRuntime,
{
    type Config = C;

    fn config(&self) -> &C {
        &self.config
    }

    async fn start<S>(&mut self, context: &mut SessionContext<S>) -> Result<(), HarnessError>
    where
        S: Sandbox,
    {
        build::start(self, context).await
    }

    fn turn<S>(
        &mut self,
        context: &mut SessionContext<S>,
        prompt: &str,
    ) -> impl Stream<Item = Result<Event, HarnessError>> + Send
    where
        S: Sandbox,
    {
        turn::turn(self, context, prompt)
    }

    fn stop<S>(
        &mut self,
        _: &mut SessionContext<S>,
    ) -> impl Future<Output = Result<(), HarnessError>> + Send
    where
        S: Sandbox,
    {
        self.graph = None;
        ready(Ok(()))
    }

    fn native_session_id(&self) -> Option<String> {
        self.thread_id.clone()
    }

    fn resume<S>(
        &mut self,
        _: &mut SessionContext<S>,
        native_session_id: &str,
    ) -> impl Future<Output = Result<(), HarnessError>> + Send
    where
        S: Sandbox,
    {
        self.thread_id = Some(native_session_id.to_owned());
        ready(Ok(()))
    }

    fn history<S>(
        &self,
        context: &SessionContext<S>,
    ) -> impl Future<Output = Result<Vec<JsonObject>, HarnessError>> + Send
    where
        S: Sync,
    {
        ready(self.history_messages(context))
    }
}
