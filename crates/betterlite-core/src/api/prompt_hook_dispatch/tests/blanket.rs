#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::prompt_contracts::{
    PromptAsyncCompileRequest, PromptManagementClient, StandardCallbackDynamicParams,
};
use betterlite_models::prompt_spec::PromptSpec;
use betterlite_prompts::orchestration::prompt_orchestration::{BoxFuture, PromptManagementBackend};
use thiserror::Error;

use crate::api::prompt_hook_dispatch::PromptManager;

#[derive(Debug, Error)]
#[error("the stub backend cannot compile prompts")]
struct StubError;

struct StubBackend {
    name: &'static str,
    runs: bool,
}

impl PromptManagementBackend for StubBackend {
    type CompileError = StubError;

    fn integration_name(&self) -> &'static str {
        self.name
    }

    fn should_run(
        &self,
        _prompt_id: Option<&str>,
        _prompt_spec: Option<&PromptSpec>,
        _dynamic_callback_params: &StandardCallbackDynamicParams,
    ) -> bool {
        self.runs
    }

    fn compile_template<'backend>(
        &'backend self,
        _request: &'backend PromptAsyncCompileRequest<'_>,
    ) -> BoxFuture<'backend, Result<PromptManagementClient, StubError>> {
        Box::pin(async { Err(StubError) })
    }
}

fn dynamic() -> StandardCallbackDynamicParams {
    Json::Object(JsonObject::new())
}

#[test]
fn the_blanket_manager_reports_the_backend_name() {
    let backend = StubBackend {
        name: "stub_integration",
        runs: false,
    };
    assert_eq!(
        <StubBackend as PromptManager<StubError>>::integration_name(&backend),
        "stub_integration"
    );
}

#[test]
fn the_blanket_manager_reports_the_backend_check() {
    let running = StubBackend {
        name: "running",
        runs: true,
    };
    assert_eq!(
        <StubBackend as PromptManager<StubError>>::should_run(
            &running,
            Some("prompt-1"),
            None,
            &dynamic()
        ),
        Ok(true)
    );
    let idle = StubBackend {
        name: "idle",
        runs: false,
    };
    assert_eq!(
        <StubBackend as PromptManager<StubError>>::should_run(&idle, None, None, &dynamic()),
        Ok(false)
    );
}
