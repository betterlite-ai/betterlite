use super::error::{CompletionCallError, into_call_error};
use super::ports::{McpLoopRunner, Sleeper};
use super::request;
use super::retry;
use crate::call_setup::RequestMap;
use crate::call_setup::correlation::{CallOutcome, ChatStream};
use crate::call_wrapper::error::CallError;
use crate::call_wrapper::ports::{IdSource, WrapperClock};
use crate::call_wrapper::state::LoggingHandle;
use crate::call_wrapper::target::CallTarget;
use crate::completion_request::args::CompletionSettings;
use crate::completion_request::call::{MockCall, ModelListCall, PreparedCall, PreparedCompletion};
use crate::completion_request::ports::CompletionPorts;
use crate::completion_request::prepare_completion;
use async_trait::async_trait;
use betterlite_config::sections::mock_response::failure::MockError;
use betterlite_config::sections::mock_response::selection::resolve_mock;
use betterlite_core::dispatch::Dispatch;
use betterlite_core::mock;
use betterlite_core::operation::Operation;
use betterlite_core::operation::chat::ChatCall;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::MockSettings;
use betterlite_redact::secret::Redactor;
use std::sync::Arc;
// main.py's completion: prepare the call, then run the outcome B5655 chose
pub struct CompletionTarget<'ports> {
    pub settings: CompletionSettings,
    pub ports: CompletionPorts<'ports>,
    pub dispatch: Arc<Dispatch>,
    pub calls: Arc<dyn ChatCall>,
    pub mock: MockSettings,
    pub ids: Arc<dyn IdSource>,
    pub clock: Arc<dyn WrapperClock>,
    pub redactor: Redactor,
    pub mcp: &'ports dyn McpLoopRunner,
    pub sleeper: &'ports dyn Sleeper,
}
impl CompletionTarget<'_> {
    pub fn error(&self, error: CompletionCallError, model: &str) -> CallError {
        into_call_error(&error, model, "", &self.redactor)
    }
    pub(crate) async fn run(
        &self,
        prepared: PreparedCompletion,
        acompletion: bool,
        logging: Option<&LoggingHandle>,
    ) -> Result<CallOutcome, CompletionCallError> {
        match prepared {
            PreparedCompletion::Dispatch(call) => self.dispatch_call(&call).await,
            PreparedCompletion::ResponsesBridge(call) => self.bridge_call(&call).await,
            PreparedCompletion::McpLoop(call) => self.mcp.run(&call, acompletion).await,
            PreparedCompletion::Mock(call) => self.mock_call(&call),
            PreparedCompletion::Fallbacks(call) => {
                super::fallbacks::run_fallbacks(self, call, logging).await
            }
            PreparedCompletion::ModelList(call) => self.model_list(call, logging).await,
        }
    }
    // The provider chain's end: palm raised its ValueError, and everything else goes to B014's dispatch
    pub(crate) async fn dispatch_call(
        &self,
        call: &PreparedCall,
    ) -> Result<CallOutcome, CompletionCallError> {
        if call.custom_llm_provider.as_deref() == Some("palm") {
            return Err(CompletionCallError::Palm);
        }
        let body = request::chat_body(call)?;
        let operation = request::chat_operation(call, &body, Arc::clone(&self.calls))?;
        request::execute(&self.dispatch, operation).await
    }
    // The Responses API bridge B5655 chose; its entry owns the request shape
    async fn bridge_call(&self, call: &PreparedCall) -> Result<CallOutcome, CompletionCallError> {
        if call.custom_llm_provider.as_deref() == Some("palm") {
            return Err(CompletionCallError::Palm);
        }
        request::execute(&self.dispatch, request::responses_operation()).await
    }
    // B255's mock calls: the answer the caller or the operator injected, never a provider
    fn mock_call(&self, call: &MockCall) -> Result<CallOutcome, CompletionCallError> {
        let response = match call.call.kwargs.get("mock_response") {
            None | Some(Json::Null) => None,
            Some(value) => Some(kwarg(value, "mock_response")?),
        };
        let tool_calls = match call.call.kwargs.get("mock_tool_calls") {
            None | Some(Json::Null) => None,
            Some(value) => Some(kwarg(value, "mock_tool_calls")?),
        };
        let choice_count = call
            .call
            .choice_count
            .and_then(|count| u32::try_from(count).ok());
        let Some(mut plan) =
            resolve_mock(None, response.as_ref(), tool_calls.as_deref(), choice_count)
        else {
            return Err(CompletionCallError::MissingMock);
        };
        plan.prompt_tokens = call
            .prompt_tokens
            .and_then(|count| u64::try_from(count).ok());
        let id = format!("chatcmpl-{}", self.ids.next_call_id());
        let created = u64::try_from(self.clock.now().timestamp()).unwrap_or(0);
        if call.call.stream {
            let mock::chat::ChatAnswer::Content(content) = mock::chat::answer(&plan)? else {
                return Err(CompletionCallError::from(MockError::ObjectStream));
            };
            let spec = mock::stream::StreamSpec {
                content,
                choice_count: plan.choice_count,
                model: &call.call.model,
                id: &id,
                created,
            };
            let chunks: Vec<_> = mock::stream::chunks(&spec).collect();
            return Ok(CallOutcome::ChatStream(ChatStream {
                model_response: serde_json::from_value(serde_json::to_value(&chunks)?)?,
                json_mode: false,
                converted_from_mock: true,
            }));
        }
        let response =
            mock::chat::response(&plan, self.mock.usage, &call.call.model, &id, created)?;
        let encoded = serde_json::to_value(&response)?;
        Ok(CallOutcome::Response(serde_json::from_value(encoded)?))
    }
    // batch_completion_models: each deployment runs with the request's params overlaid by its own,
    // and a failing deployment is skipped for the next one
    async fn model_list(
        &self,
        call: ModelListCall,
        logging: Option<&LoggingHandle>,
    ) -> Result<CallOutcome, CompletionCallError> {
        let mut last_failure: Option<CompletionCallError> = None;
        let mut request = request::args_to_map(&call.args)?;
        request.shift_remove("model_list");
        for deployment in &call.deployments {
            for (name, value) in deployment {
                request.insert(name.clone(), value.clone());
            }
            match self.call(request.clone(), logging).await {
                Ok(outcome) if !super::fallbacks::is_empty(&outcome) => return Ok(outcome),
                Ok(_) => {}
                Err(error) => last_failure = Some(CompletionCallError::Call(error)),
            }
        }
        Err(last_failure.unwrap_or(CompletionCallError::NoModelListAnswer))
    }
}
#[async_trait]
impl CallTarget for CompletionTarget<'_> {
    fn name(&self) -> &str {
        "completion"
    }
    async fn call(
        &self,
        request: RequestMap,
        logging: Option<&LoggingHandle>,
    ) -> Result<CallOutcome, CallError> {
        let args = request::args_from_map(&request).map_err(|error| self.error(error, ""))?;
        let model = args.model.clone().unwrap_or_default();
        let prepared = prepare_completion(&args, &self.settings, &self.ports)
            .await
            .map_err(|error| self.error(CompletionCallError::from(error), &model))?;
        self.run(prepared, false, logging)
            .await
            .map_err(|error| self.error(error, &model))
    }
    async fn retry(
        &self,
        request: RequestMap,
        logging: Option<&LoggingHandle>,
    ) -> Result<CallOutcome, CallError> {
        retry::with_retries(self, request, logging, self.sleeper).await
    }
}
fn kwarg<T: serde::de::DeserializeOwned>(
    value: &Json,
    field: &'static str,
) -> Result<T, CompletionCallError> {
    let encoded = serde_json::to_value(value)?;
    serde_json::from_value(encoded).map_err(|_error| CompletionCallError::Field { field })
}
