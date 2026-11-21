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
use crate::completion_request::args::{CompletionArgs, CompletionSettings};
use crate::completion_request::call::{MockCall, ModelListCall, PreparedCall, PreparedCompletion};
use crate::completion_request::ports::CompletionPorts;
use crate::completion_request::prepare_completion;
use async_trait::async_trait;
use betterlite_config::sections::mock_response::failure::MockError;
use betterlite_config::sections::mock_response::selection::resolve_mock;
use betterlite_config::sections::mock_response::{MockResponse, MockToolCall};
use betterlite_core::dispatch::Dispatch;
use betterlite_core::mock;
use betterlite_core::operation::Operation;
use betterlite_core::operation::chat::ChatCall;
use betterlite_core::operation::responses::ResponsesRequest;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::MockSettings;
use betterlite_redact::secret::Redactor;
use futures_util::StreamExt as _;
use futures_util::stream;
use serde::de::DeserializeOwned;
use std::sync::Arc;
pub(crate) const MAX_MODEL_LIST_DEPLOYMENTS: usize = 100;
const MODEL_LIST_CONCURRENCY: usize = 10;
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
    pub stream_bytes: usize,
}
impl CompletionTarget<'_> {
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
                super::fallbacks::run_fallbacks(self, call, logging, self.ids.as_ref()).await
            }
            PreparedCompletion::ModelList(call) => self.model_list(call, logging).await,
        }
    }
    pub(crate) async fn dispatch_call(
        &self,
        call: &PreparedCall,
    ) -> Result<CallOutcome, CompletionCallError> {
        if call.custom_llm_provider.as_deref() == Some("palm") {
            return Err(CompletionCallError::Palm);
        }
        let body = request::chat_body(call)?;
        let operation =
            request::chat_operation(call, &body, Arc::clone(&self.calls), self.ids.as_ref())?;
        request::execute(
            &self.dispatch,
            operation,
            call.timeout.as_ref(),
            self.stream_bytes,
        )
        .await
    }
    async fn bridge_call(&self, call: &PreparedCall) -> Result<CallOutcome, CompletionCallError> {
        if call.custom_llm_provider.as_deref() == Some("palm") {
            return Err(CompletionCallError::Palm);
        }
        request::execute(
            &self.dispatch,
            Operation::Responses(ResponsesRequest),
            call.timeout.as_ref(),
            self.stream_bytes,
        )
        .await
    }
    fn mock_call(&self, call: &MockCall) -> Result<CallOutcome, CompletionCallError> {
        let mock_response: Option<MockResponse> = optional_kwarg(&call.call, "mock_response")?;
        let tool_calls: Option<Vec<MockToolCall>> = optional_kwarg(&call.call, "mock_tool_calls")?;
        let choice_count = call
            .call
            .choice_count
            .and_then(|count| u32::try_from(count).ok());
        let Some(mut plan) = resolve_mock(
            None,
            mock_response.as_ref(),
            tool_calls.as_deref(),
            choice_count,
        ) else {
            return Err(CompletionCallError::MissingMock);
        };
        let prompt_tokens = call.prompt_tokens;
        plan.prompt_tokens = prompt_tokens.and_then(|count| u64::try_from(count).ok());
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
    async fn model_list(
        &self,
        call: ModelListCall,
        logging: Option<&LoggingHandle>,
    ) -> Result<CallOutcome, CompletionCallError> {
        let deployment_count = call.deployments.len();
        if deployment_count > MAX_MODEL_LIST_DEPLOYMENTS {
            return Err(CompletionCallError::TooManyDeployments {
                count: deployment_count,
                maximum: MAX_MODEL_LIST_DEPLOYMENTS,
            });
        }
        let mut base = request::args_to_map(&call.args)?;
        base.shift_remove("model_list");
        let mut requests = Vec::with_capacity(deployment_count);
        for entry in &call.deployments {
            for (name, value) in entry {
                base.insert(name.clone(), value.clone());
            }
            requests.push(base.clone());
        }
        let mut attempts = stream::iter(
            requests
                .into_iter()
                .map(|attempt| self.call(attempt, logging)),
        )
        .buffer_unordered(MODEL_LIST_CONCURRENCY);
        let mut last_failure: Option<CompletionCallError> = None;
        while let Some(result) = attempts.next().await {
            match result {
                Ok(outcome) if matches!(&outcome, CallOutcome::Response(Json::Null)) => {}
                Ok(outcome) => return Ok(outcome),
                Err(error) => last_failure = Some(CompletionCallError::Call(error)),
            }
        }
        Err(last_failure.unwrap_or(CompletionCallError::NoModelListAnswer))
    }
}
#[async_trait]
impl CallTarget for CompletionTarget<'_> {
    fn name(&self) -> &'static str {
        "completion"
    }
    async fn call(
        &self,
        request: RequestMap,
        logging: Option<&LoggingHandle>,
    ) -> Result<CallOutcome, CallError> {
        let encoded = serde_json::to_value(&request).map_err(|error| {
            into_call_error(&CompletionCallError::from(error), "", "", &self.redactor)
        })?;
        let args: CompletionArgs = serde_json::from_value(encoded).map_err(|error| {
            into_call_error(&CompletionCallError::from(error), "", "", &self.redactor)
        })?;
        let model = args.model.clone().unwrap_or_default();
        let prepared = prepare_completion(&args, &self.settings, &self.ports)
            .await
            .map_err(|error| {
                into_call_error(
                    &CompletionCallError::from(error),
                    &model,
                    "",
                    &self.redactor,
                )
            })?;
        Box::pin(self.run(prepared, false, logging))
            .await
            .map_err(|error| into_call_error(&error, &model, "", &self.redactor))
    }
    async fn retry(
        &self,
        request: RequestMap,
        logging: Option<&LoggingHandle>,
    ) -> Result<CallOutcome, CallError> {
        retry::with_retries(self, request, logging, self.sleeper).await
    }
}
fn optional_kwarg<T>(
    call: &PreparedCall,
    name: &'static str,
) -> Result<Option<T>, CompletionCallError>
where
    T: DeserializeOwned,
{
    let value = match call.kwargs.get(name) {
        None | Some(Json::Null) => return Ok(None),
        Some(value) => value,
    };
    let encoded = serde_json::to_value(value)?;
    serde_json::from_value(encoded).map_err(|_error| CompletionCallError::Field { field: name })
}
