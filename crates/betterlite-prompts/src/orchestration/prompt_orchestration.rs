use std::error::Error;
use std::future::Future;
use std::pin::Pin;

use betterlite_models::prompt_contracts::{
    AllMessageValues, PromptAsyncCompileRequest, PromptIgnoreFlags, PromptManagementClient,
    PromptParams, PromptProcessingOptions, StandardCallbackDynamicParams,
    post_compile_prompt_processing, resolve_ignore_flags,
};
use betterlite_models::prompt_spec::PromptSpec;
use thiserror::Error;

pub type BoxFuture<'future, Output> = Pin<Box<dyn Future<Output = Output> + Send + 'future>>;
pub type CompileFuture<'backend, BackendError> =
    BoxFuture<'backend, Result<PromptManagementClient, PromptOrchestrationError<BackendError>>>;
pub type CompletionFuture<'backend, BackendError> =
    BoxFuture<'backend, Result<ChatCompletionPrompt, PromptOrchestrationError<BackendError>>>;

pub struct CompilePromptCall<'request> {
    pub prompt_id: &'request str,
    pub client_messages: Vec<AllMessageValues>,
    pub prompt_variables: Option<&'request PromptParams>,
    pub dynamic_callback_params: &'request StandardCallbackDynamicParams,
    pub prompt_spec: Option<&'request PromptSpec>,
    pub prompt_label: Option<&'request str>,
    pub prompt_version: Option<i64>,
}

pub struct AsyncCompilePromptCall<'request> {
    pub prompt_id: Option<&'request str>,
    pub client_messages: Vec<AllMessageValues>,
    pub prompt_variables: Option<&'request PromptParams>,
    pub dynamic_callback_params: &'request StandardCallbackDynamicParams,
    pub prompt_spec: Option<&'request PromptSpec>,
    pub prompt_label: Option<&'request str>,
    pub prompt_version: Option<i64>,
}

pub struct ChatCompletionPromptRequest<'request> {
    pub model: &'request str,
    pub messages: Vec<AllMessageValues>,
    pub non_default_params: PromptParams,
    pub prompt_id: Option<&'request str>,
    pub prompt_variables: Option<&'request PromptParams>,
    pub dynamic_callback_params: &'request StandardCallbackDynamicParams,
    pub prompt_spec: Option<&'request PromptSpec>,
    pub prompt_label: Option<&'request str>,
    pub prompt_version: Option<i64>,
    pub ignore_flags: PromptIgnoreFlags,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ChatCompletionPrompt {
    pub model: String,
    pub messages: Vec<AllMessageValues>,
    pub non_default_params: PromptParams,
}

impl<'request> ChatCompletionPromptRequest<'request> {
    fn async_compile_call(&self) -> AsyncCompilePromptCall<'request> {
        AsyncCompilePromptCall {
            prompt_id: self.prompt_id,
            client_messages: self.messages.clone(),
            prompt_variables: self.prompt_variables,
            dynamic_callback_params: self.dynamic_callback_params,
            prompt_spec: self.prompt_spec,
            prompt_label: self.prompt_label,
            prompt_version: self.prompt_version,
        }
    }

    // LiteLLM's sync hook compiles without the prompt spec and only then reads it for the ignore flags.
    fn sync_compile_call(&self, prompt_id: &'request str) -> CompilePromptCall<'request> {
        CompilePromptCall {
            prompt_id,
            client_messages: self.messages.clone(),
            prompt_variables: self.prompt_variables,
            dynamic_callback_params: self.dynamic_callback_params,
            prompt_spec: None,
            prompt_label: self.prompt_label,
            prompt_version: self.prompt_version,
        }
    }
}

impl<'request> From<CompilePromptCall<'request>> for AsyncCompilePromptCall<'request> {
    fn from(call: CompilePromptCall<'request>) -> Self {
        Self {
            prompt_id: Some(call.prompt_id),
            client_messages: call.client_messages,
            prompt_variables: call.prompt_variables,
            dynamic_callback_params: call.dynamic_callback_params,
            prompt_spec: call.prompt_spec,
            prompt_label: call.prompt_label,
            prompt_version: call.prompt_version,
        }
    }
}

#[derive(Debug, Error)]
pub enum PromptOrchestrationError<BackendError>
where
    BackendError: Error + 'static,
{
    #[error("Error compiling prompt: {source}. Prompt id={prompt_id}")]
    Compile {
        prompt_id: String,
        #[source]
        source: BackendError,
    },
}

pub trait PromptManagementBackend: Sync {
    type CompileError: Error + Send + Sync + 'static;

    fn integration_name(&self) -> &'static str;

    // LiteLLM's should_run_prompt_management; the backends already own that name as an inherent method.
    fn should_run(
        &self,
        prompt_id: Option<&str>,
        prompt_spec: Option<&PromptSpec>,
        dynamic_callback_params: &StandardCallbackDynamicParams,
    ) -> bool;

    // LiteLLM's async_compile_prompt_helper; the backends already own that name as an inherent method.
    fn compile_template<'backend>(
        &'backend self,
        request: &'backend PromptAsyncCompileRequest<'_>,
    ) -> BoxFuture<'backend, Result<PromptManagementClient, Self::CompileError>>;

    // The sync flow awaits the backend helper because every betterlite prompt transport is async.
    fn compile_prompt<'backend>(
        &'backend self,
        call: CompilePromptCall<'backend>,
    ) -> CompileFuture<'backend, Self::CompileError> {
        let asynchronous = AsyncCompilePromptCall::from(call);
        self.async_compile_prompt(asynchronous)
    }

    fn async_compile_prompt<'backend>(
        &'backend self,
        call: AsyncCompilePromptCall<'backend>,
    ) -> CompileFuture<'backend, Self::CompileError> {
        Box::pin(compile_prompt_async(self, call))
    }

    fn get_chat_completion_prompt<'backend>(
        &'backend self,
        request: ChatCompletionPromptRequest<'backend>,
    ) -> CompletionFuture<'backend, Self::CompileError> {
        sync_completion_prompt(self, request)
    }

    // LiteLLM's async_get_chat_completion_prompt; the 30-character name rule shortens it.
    fn async_completion_prompt<'backend>(
        &'backend self,
        request: ChatCompletionPromptRequest<'backend>,
    ) -> CompletionFuture<'backend, Self::CompileError> {
        Box::pin(async move {
            if !self.should_run(
                request.prompt_id,
                request.prompt_spec,
                request.dynamic_callback_params,
            ) {
                return Ok(unchanged_prompt(request));
            }
            let call = request.async_compile_call();
            let compiled = self.async_compile_prompt(call).await?;
            Ok(finish_chat_completion_prompt(
                self.integration_name(),
                &compiled,
                request,
            ))
        })
    }
}

// LiteLLM's sync hook returns before it checks eligibility or compiles anything.
pub fn sync_completion_prompt<'backend, Backend>(
    backend: &'backend Backend,
    request: ChatCompletionPromptRequest<'backend>,
) -> CompletionFuture<'backend, Backend::CompileError>
where
    Backend: PromptManagementBackend + ?Sized,
{
    Box::pin(async move {
        let Some(resolved_id) = request.prompt_id else {
            return Ok(unchanged_prompt(request));
        };
        if !backend.should_run(
            Some(resolved_id),
            request.prompt_spec,
            request.dynamic_callback_params,
        ) {
            return Ok(unchanged_prompt(request));
        }
        let call = request.sync_compile_call(resolved_id);
        let compiled = backend.compile_prompt(call).await?;
        Ok(finish_chat_completion_prompt(
            backend.integration_name(),
            &compiled,
            request,
        ))
    })
}

fn unchanged_prompt(request: ChatCompletionPromptRequest<'_>) -> ChatCompletionPrompt {
    ChatCompletionPrompt {
        model: request.model.to_owned(),
        messages: request.messages,
        non_default_params: request.non_default_params,
    }
}

fn finish_chat_completion_prompt(
    integration_name: &'static str,
    compiled: &PromptManagementClient,
    request: ChatCompletionPromptRequest<'_>,
) -> ChatCompletionPrompt {
    let ignore_flags = resolve_ignore_flags(request.prompt_spec, request.ignore_flags);
    let (prompt_model, completed_messages, updated_params) = post_compile_prompt_processing(
        compiled,
        request.messages,
        request.non_default_params,
        request.model.to_owned(),
        &PromptProcessingOptions {
            integration_name: integration_name.to_owned(),
            ignore_flags,
        },
    );
    ChatCompletionPrompt {
        model: prompt_model,
        messages: completed_messages,
        non_default_params: updated_params,
    }
}

async fn compile_prompt_async<Backend>(
    backend: &Backend,
    call: AsyncCompilePromptCall<'_>,
) -> Result<PromptManagementClient, PromptOrchestrationError<Backend::CompileError>>
where
    Backend: PromptManagementBackend + ?Sized,
{
    let compiled = backend
        .compile_template(&PromptAsyncCompileRequest {
            prompt_id: call.prompt_id,
            prompt_variables: call.prompt_variables,
            dynamic_callback_params: call.dynamic_callback_params,
            prompt_spec: call.prompt_spec,
            prompt_label: call.prompt_label,
            prompt_version: call.prompt_version,
        })
        .await;
    complete_compiled_prompt(
        call.prompt_id.unwrap_or("None"),
        compiled,
        call.client_messages,
    )
}

fn complete_compiled_prompt<BackendError>(
    prompt_id: &str,
    compiled: Result<PromptManagementClient, BackendError>,
    client_messages: Vec<AllMessageValues>,
) -> Result<PromptManagementClient, PromptOrchestrationError<BackendError>>
where
    BackendError: Error + 'static,
{
    let mut client = compiled.map_err(|source| PromptOrchestrationError::Compile {
        prompt_id: prompt_id.to_owned(),
        source,
    })?;
    let mut completed = client.prompt_template.clone();
    completed.extend(client_messages);
    client.completed_messages = Some(completed);
    Ok(client)
}

mod tests;
