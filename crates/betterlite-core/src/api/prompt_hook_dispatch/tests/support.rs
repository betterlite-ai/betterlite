use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::prompt_contracts::StandardCallbackDynamicParams;
use betterlite_models::prompt_spec::PromptSpec;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use betterlite_prompts::orchestration::prompt_orchestration::{
    ChatCompletionPrompt, ChatCompletionPromptRequest, CompletionFuture, PromptOrchestrationError,
};
use betterlite_redact::secret::Redactor;
use chrono::DateTime;
use thiserror::Error;

use crate::api::prompt_hook_dispatch::{CompatibleLoggerFactory, PromptCheckError, PromptManager};
use crate::api::vector_store_registry::store::VectorStoreRegistry;

pub(crate) type Backend = Arc<dyn PromptManager<TestError> + Send + Sync>;
pub(crate) type Backends = Vec<Backend>;

#[derive(Default)]
pub(crate) struct Lists {
    pub(crate) managers: Backends,
    pub(crate) callbacks: Backends,
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub(crate) enum TestError {
    #[error("the prompt manager failed")]
    Failed,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum HookBehavior {
    Echo,
    AddBreakpoint,
    Fail,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RecordedCall {
    pub(crate) model: String,
    pub(crate) messages: Vec<Json>,
    pub(crate) non_default_params: JsonObject,
    pub(crate) prompt_id: Option<String>,
    pub(crate) prompt_variables: Option<JsonObject>,
    pub(crate) dynamic_callback_params: Json,
    pub(crate) prompt_label: Option<String>,
    pub(crate) prompt_version: Option<i64>,
    pub(crate) asynchronous: bool,
}

pub(crate) struct FakeManager {
    name: &'static str,
    check: Result<bool, PromptCheckError>,
    behavior: HookBehavior,
    checks: Mutex<Vec<Option<String>>>,
    calls: Mutex<Vec<RecordedCall>>,
}

impl FakeManager {
    pub(crate) fn new(
        name: &'static str,
        check: Result<bool, PromptCheckError>,
        behavior: HookBehavior,
    ) -> Self {
        Self {
            name,
            check,
            behavior,
            checks: Mutex::new(Vec::new()),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn checks(&self) -> Vec<Option<String>> {
        self.checks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn calls(&self) -> Vec<RecordedCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record_check(&self, prompt_id: Option<&str>) {
        self.checks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(prompt_id.map(str::to_owned));
    }

    fn record_call(&self, request: &ChatCompletionPromptRequest<'_>, asynchronous: bool) {
        let call = RecordedCall {
            model: request.model.to_owned(),
            messages: request.messages.clone(),
            non_default_params: request.non_default_params.clone(),
            prompt_id: request.prompt_id.map(str::to_owned),
            prompt_variables: request.prompt_variables.cloned(),
            dynamic_callback_params: request.dynamic_callback_params.clone(),
            prompt_label: request.prompt_label.map(str::to_owned),
            prompt_version: request.prompt_version,
            asynchronous,
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }

    fn respond_with<'request>(
        &'request self,
        request: ChatCompletionPromptRequest<'request>,
        asynchronous: bool,
    ) -> CompletionFuture<'request, TestError> {
        self.record_call(&request, asynchronous);
        let behavior = self.behavior;
        Box::pin(async move { respond(behavior, request) })
    }
}

impl PromptManager<TestError> for FakeManager {
    fn integration_name(&self) -> &'static str {
        self.name
    }

    fn should_run(
        &self,
        prompt_id: Option<&str>,
        _prompt_spec: Option<&PromptSpec>,
        _dynamic_callback_params: &StandardCallbackDynamicParams,
    ) -> Result<bool, PromptCheckError> {
        self.record_check(prompt_id);
        self.check
    }

    fn get_chat_completion_prompt<'request>(
        &'request self,
        request: ChatCompletionPromptRequest<'request>,
    ) -> CompletionFuture<'request, TestError> {
        self.respond_with(request, false)
    }

    fn async_completion_prompt<'request>(
        &'request self,
        request: ChatCompletionPromptRequest<'request>,
    ) -> CompletionFuture<'request, TestError> {
        self.respond_with(request, true)
    }
}

pub(crate) fn backend(fake: &Arc<FakeManager>) -> Backend {
    Arc::<FakeManager>::clone(fake)
}

fn respond(
    behavior: HookBehavior,
    request: ChatCompletionPromptRequest<'_>,
) -> Result<ChatCompletionPrompt, PromptOrchestrationError<TestError>> {
    match behavior {
        HookBehavior::Echo => Ok(ChatCompletionPrompt {
            model: request.model.to_owned(),
            messages: request.messages,
            non_default_params: request.non_default_params,
        }),
        HookBehavior::AddBreakpoint => {
            let mut messages = request.messages;
            messages.push(breakpoint_message());
            Ok(ChatCompletionPrompt {
                model: request.model.to_owned(),
                messages,
                non_default_params: request.non_default_params,
            })
        }
        HookBehavior::Fail => Err(PromptOrchestrationError::Compile {
            prompt_id: request.prompt_id.unwrap_or("None").to_owned(),
            source: TestError::Failed,
        }),
    }
}

fn breakpoint_message() -> Json {
    serde_json::from_str(
        r#"{"role":"user","content":[{"type":"text","text":"mark","cache_control":{"type":"ephemeral"}}]}"#,
    )
    .expect("a breakpoint message")
}

pub(crate) struct FakeFactory {
    known: Vec<&'static str>,
    managers: Vec<(&'static str, Backend)>,
    inits: Mutex<Vec<String>>,
}

impl FakeFactory {
    pub(crate) fn new(known: Vec<&'static str>, managers: Vec<(&'static str, Backend)>) -> Self {
        Self {
            known,
            managers,
            inits: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn inits(&self) -> Vec<String> {
        self.inits
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CompatibleLoggerFactory<TestError> for FakeFactory {
    fn known_callbacks(&self) -> &[&'static str] {
        &self.known
    }

    fn init(&self, name: &str) -> Option<Backend> {
        self.inits
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(name.to_owned());
        self.managers
            .iter()
            .find(|(known, _)| *known == name)
            .map(|(_, manager)| Arc::clone(manager))
    }
}

pub(crate) fn logging_state() -> LoggingState {
    let init = LoggingInit {
        model: "gpt-4".to_owned(),
        call_type: "completion".to_owned(),
        start_time: DateTime::parse_from_rfc3339("2024-01-01T00:00:00+00:00")
            .expect("a valid timestamp"),
        litellm_call_id: "call-1".to_owned(),
        function_id: "function-1".to_owned(),
        messages: None,
        stream: Some(false),
        litellm_trace_id: Some("trace-1".to_owned()),
        kwargs: JsonObject::new(),
        litellm_params: JsonObject::new(),
        callback_dynamic_params: JsonObject::new(),
        trusted_callback_vars: Vec::new(),
        standard_built_in_tools_params: JsonObject::new(),
        dynamic_callbacks: DynamicCallbacks::default(),
        applied_guardrails: None,
        request_logging: RequestLoggingFlags::default(),
        callback_policy: CallbackPolicy::default(),
    };
    let redactor = Redactor::new(16).expect("a valid redactor");
    let mut context = CorrelationContext::default();
    let setup = CorrelationSetup {
        context: &mut context,
        settings: CorrelationSettings::default(),
    };
    LoggingState::new(
        init,
        setup,
        &redactor,
        || "generated-trace".to_owned(),
        || Duration::ZERO,
    )
    .expect("a valid logging state")
}

pub(crate) fn registry(stores: Vec<LiteLlmManagedVectorStore>) -> VectorStoreRegistry {
    VectorStoreRegistry {
        stores,
        ..VectorStoreRegistry::default()
    }
}

pub(crate) fn store(id: &str) -> LiteLlmManagedVectorStore {
    LiteLlmManagedVectorStore {
        vector_store_id: Some(id.to_owned()),
        custom_llm_provider: Some("openai".to_owned()),
        vector_store_name: None,
        vector_store_description: None,
        vector_store_metadata: None,
        created_at: None,
        updated_at: None,
        litellm_credential_name: None,
        litellm_params: None,
        team_id: None,
        user_id: None,
        is_config: None,
        extra: BTreeMap::new(),
    }
}

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("a JSON literal")
}

pub(crate) fn object(entries: Vec<(&str, Json)>) -> JsonObject {
    let mut fields = JsonObject::new();
    for (name, value) in entries {
        fields.insert(name.to_owned(), value);
    }
    fields
}

pub(crate) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(crate) fn list(values: &[&str]) -> Json {
    Json::List(values.iter().copied().map(text).collect())
}
