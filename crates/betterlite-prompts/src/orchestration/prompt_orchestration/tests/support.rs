use std::sync::Mutex;

use betterlite_models::decisions::Json as ContractJson;
use betterlite_models::prompt_contracts::{
    AllMessageValues, PromptAsyncCompileRequest, PromptIgnoreFlags, PromptManagementClient,
    PromptParams, StandardCallbackDynamicParams,
};
use betterlite_models::prompt_spec::PromptSpec;
use indexmap::IndexMap;
use thiserror::Error;

use crate::orchestration::prompt_orchestration::{
    BoxFuture, ChatCompletionPromptRequest, PromptManagementBackend,
};

pub(crate) fn json(text: &str) -> ContractJson {
    serde_json::from_str(text).expect("valid JSON")
}

pub(crate) fn params(text: &str) -> PromptParams {
    serde_json::from_str(text).expect("valid prompt params")
}

pub(crate) fn message(role: &str, content: &str) -> AllMessageValues {
    let mut fields = IndexMap::new();
    fields.insert("role".to_owned(), ContractJson::Text(role.to_owned()));
    fields.insert("content".to_owned(), ContractJson::Text(content.to_owned()));
    ContractJson::Object(fields)
}

pub(crate) fn client_messages() -> Vec<AllMessageValues> {
    vec![message("user", "client question")]
}

pub(crate) fn ignoring_all() -> PromptIgnoreFlags {
    PromptIgnoreFlags {
        model: true,
        optional_params: true,
    }
}

pub(crate) fn prompt_spec(ignore_flags: PromptIgnoreFlags) -> PromptSpec {
    let text = format!(
        r#"{{"prompt_id":"greeting","litellm_params":{{"prompt_integration":"dotprompt","ignore_prompt_manager_model":{},"ignore_prompt_manager_optional_params":{}}}}}"#,
        ignore_flags.model, ignore_flags.optional_params
    );
    serde_json::from_str(&text).expect("prompt spec")
}

pub(crate) fn compiled_client() -> PromptManagementClient {
    PromptManagementClient {
        prompt_id: Some("greeting".to_owned()),
        prompt_template: vec![message("system", "template prompt")],
        prompt_template_model: Some("template-model".to_owned()),
        prompt_optional_params: Some(params(r#"{"temperature":0.8}"#)),
        completed_messages: None,
    }
}

pub(crate) fn request<'request>(
    prompt_id: Option<&'request str>,
    model: &'request str,
    dynamic_callback_params: &'request StandardCallbackDynamicParams,
) -> ChatCompletionPromptRequest<'request> {
    ChatCompletionPromptRequest {
        model,
        messages: client_messages(),
        non_default_params: params(r#"{"temperature":0.2,"max_tokens":10}"#),
        prompt_id,
        prompt_variables: None,
        dynamic_callback_params,
        prompt_spec: None,
        prompt_label: None,
        prompt_version: None,
        ignore_flags: PromptIgnoreFlags::default(),
    }
}

#[derive(Debug, Error)]
#[error("backend compile failed")]
pub(crate) struct FakeFailure;

#[derive(Clone, Debug)]
pub(crate) struct RecordedCompile {
    pub(crate) id: Option<String>,
    pub(crate) spec: Option<PromptSpec>,
    pub(crate) label: Option<String>,
    pub(crate) version: Option<i64>,
}

pub(crate) struct FakeBackend {
    integration: &'static str,
    eligible: bool,
    compiled: Mutex<Option<Result<PromptManagementClient, FakeFailure>>>,
    gate_calls: Mutex<Vec<Option<String>>>,
    compile_calls: Mutex<Vec<RecordedCompile>>,
}

impl FakeBackend {
    pub(crate) fn ready(compiled: PromptManagementClient) -> Self {
        Self::new(true, Ok(compiled))
    }

    pub(crate) fn ineligible(compiled: PromptManagementClient) -> Self {
        Self::new(false, Ok(compiled))
    }

    pub(crate) fn failing() -> Self {
        Self::new(true, Err(FakeFailure))
    }

    fn new(eligible: bool, compiled: Result<PromptManagementClient, FakeFailure>) -> Self {
        Self {
            integration: "fake",
            eligible,
            compiled: Mutex::new(Some(compiled)),
            gate_calls: Mutex::new(Vec::new()),
            compile_calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn gate_calls(&self) -> Vec<Option<String>> {
        self.gate_calls.lock().expect("gate lock").clone()
    }

    pub(crate) fn compile_calls(&self) -> Vec<RecordedCompile> {
        self.compile_calls.lock().expect("compile lock").clone()
    }
}

impl PromptManagementBackend for FakeBackend {
    type CompileError = FakeFailure;

    fn integration_name(&self) -> &'static str {
        self.integration
    }

    fn should_run(
        &self,
        prompt_id: Option<&str>,
        _prompt_spec: Option<&PromptSpec>,
        _dynamic_callback_params: &StandardCallbackDynamicParams,
    ) -> bool {
        self.gate_calls
            .lock()
            .expect("gate lock")
            .push(prompt_id.map(str::to_owned));
        self.eligible
    }

    fn compile_template<'backend>(
        &'backend self,
        request: &'backend PromptAsyncCompileRequest<'_>,
    ) -> BoxFuture<'backend, Result<PromptManagementClient, FakeFailure>> {
        self.compile_calls
            .lock()
            .expect("compile lock")
            .push(RecordedCompile {
                id: request.prompt_id.map(str::to_owned),
                spec: request.prompt_spec.cloned(),
                label: request.prompt_label.map(str::to_owned),
                version: request.prompt_version,
            });
        let compiled = self
            .compiled
            .lock()
            .expect("compiled lock")
            .take()
            .expect("a prepared compiled prompt");
        Box::pin(async move { compiled })
    }
}
