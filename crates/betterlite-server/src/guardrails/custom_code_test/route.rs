use std::error::Error as StdError;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::Extension;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use axum::routing::post;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::custom_code::runtime::CallArguments;
use betterlite_guardrails::custom_code::runtime::CallOutput;
use betterlite_guardrails::custom_code::runtime::LoadFailure as ModuleLoadFailure;
use betterlite_guardrails::custom_code::runtime::ModuleHost;
use betterlite_guardrails::custom_code::runtime::RuntimeFailure;
use betterlite_guardrails::custom_code_execution::execution::error::ExecutionError;
use betterlite_guardrails::custom_code_execution::execution::error::SandboxError;
use betterlite_guardrails::custom_code_execution::execution::eventloop::await_with_timeout;
use betterlite_guardrails::custom_code_execution::execution::offloop::call_off_loop_with_timeout;
use betterlite_guardrails::custom_code_execution::sandbox::DEFAULT_FILENAME;
use betterlite_guardrails::custom_code_execution::sandbox::compile_sandboxed;
use betterlite_guardrails::custom_code_execution::sandbox::error::CompileError;
use betterlite_guardrails::custom_code_execution::sandbox::globals::build_sandbox_globals;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use serde::Serialize;
use thiserror::Error;

use crate::auth::auth_identity::AuthContext;
use crate::error_body::json_error;

use super::body::decode_test_request;
use super::run::CallOutcome;
use super::run::CallProblem;
use super::run::CodeSandbox;
use super::run::EXECUTION_TIMEOUT;
use super::run::LoadOutcome;
use super::run::LoadProblem;
use super::run::LoadedCode;
use super::run::TestCall;
use super::run::run_test;

const ADMIN_REQUIRED: &str = "Admin access required to test custom code guardrails";
const MASTER_KEY_SUBJECT: &str = "master_key";
const UNAUTHENTICATED: &str = "the API key is not valid";

pub struct SandboxedCode {
    pub host: Arc<dyn ModuleHost>,
}

#[derive(Debug, Error)]
enum SandboxLoad {
    #[error(transparent)]
    Compile(CompileError),
    #[error(transparent)]
    Module(ModuleLoadFailure),
}

#[derive(Clone)]
struct TestRouteState {
    sandbox: Arc<dyn CodeSandbox>,
}

#[derive(Serialize)]
struct AdminDetail {
    detail: &'static str,
}

pub fn routes(sandbox: Arc<dyn CodeSandbox>) -> Router {
    Router::new()
        .route("/guardrails/test_custom_code", post(test_custom_code))
        .with_state(TestRouteState { sandbox })
}

async fn test_custom_code(
    State(state): State<TestRouteState>,
    caller: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Response {
    let Some(Extension(context)) = caller else {
        return json_error(
            StatusCode::UNAUTHORIZED,
            UNAUTHENTICATED,
            "auth_error",
            "401",
        );
    };
    let request = match decode_test_request(&body) {
        Ok(request) => request,
        Err(error) => return error.into_response(),
    };
    if !is_proxy_admin(&context) {
        return forbidden();
    }
    let response = run_test(request, state.sandbox.as_ref()).await;
    (StatusCode::OK, Json(response)).into_response()
}

fn is_proxy_admin(caller: &AuthContext) -> bool {
    caller.principal.subject == MASTER_KEY_SUBJECT
        || caller.principal.roles.contains(&Role::PlatformAdmin)
}

fn forbidden() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(AdminDetail {
            detail: ADMIN_REQUIRED,
        }),
    )
        .into_response()
}

#[async_trait]
impl CodeSandbox for SandboxedCode {
    async fn load(&self, source: &str) -> LoadOutcome {
        let globals = build_sandbox_globals();
        let owned_source = source.to_owned();
        let host = Arc::clone(&self.host);
        let loaded = call_off_loop_with_timeout(
            move || {
                let program = compile_sandboxed(&owned_source, DEFAULT_FILENAME)
                    .map_err(|error| SandboxError::Failed(SandboxLoad::Compile(error)))?;
                host.load(&program, &globals).map_err(module_error)
            },
            EXECUTION_TIMEOUT,
            "test:load",
        )
        .await;
        match loaded {
            Ok(module) => LoadOutcome::Loaded(LoadedCode(module)),
            Err(failure) => LoadOutcome::Failed(match execution_failure(failure) {
                ExecutionFailure::Timeout => LoadProblem::Timeout,
                ExecutionFailure::Exit(message) => LoadProblem::Compile(message),
                ExecutionFailure::Unexpected(message) => LoadProblem::Unexpected(message),
                ExecutionFailure::Failed(SandboxLoad::Compile(error)) => {
                    LoadProblem::Syntax(error.to_string())
                }
                ExecutionFailure::Failed(SandboxLoad::Module(module)) => module_problem(module),
            }),
        }
    }

    async fn call(&self, code: LoadedCode, arguments: TestCall) -> CallOutcome {
        let started = Instant::now();
        let module = code.0;
        // B3802 carries only request or response, so another text reaches the function as request
        let call_arguments = CallArguments {
            inputs: GuardrailValue::Object(arguments.inputs),
            request_data: GuardrailValue::Object(arguments.request_data),
            input_type: if arguments.input_type == "response" {
                GuardrailInputType::Response
            } else {
                GuardrailInputType::Request
            },
        };
        let output = match call_off_loop_with_timeout(
            move || module.call(call_arguments),
            EXECUTION_TIMEOUT,
            "test",
        )
        .await
        {
            Ok(output) => output,
            Err(error) => return CallOutcome::Failed(call_problem(error)),
        };
        let pending = match output {
            CallOutput::Value(value) => return CallOutcome::Result(value),
            CallOutput::Pending(pending) => pending,
        };
        let remaining = EXECUTION_TIMEOUT.saturating_sub(started.elapsed());
        match await_with_timeout(pending, remaining, "test").await {
            Ok(value) => CallOutcome::Result(value),
            Err(error) => CallOutcome::Failed(call_problem(error)),
        }
    }
}

fn module_error(error: SandboxError<ModuleLoadFailure>) -> SandboxError<SandboxLoad> {
    match error {
        SandboxError::Interrupted(interrupted) => SandboxError::Interrupted(interrupted),
        SandboxError::Exit(exit) => SandboxError::Exit(exit),
        SandboxError::Failed(failure) => SandboxError::Failed(SandboxLoad::Module(failure)),
    }
}

enum ExecutionFailure<E> {
    Timeout,
    Exit(String),
    Failed(E),
    Unexpected(String),
}

fn execution_failure<E>(source: ExecutionError<E>) -> ExecutionFailure<E>
where
    E: StdError,
{
    match source {
        ExecutionError::Timeout(_) | ExecutionError::Sandbox(SandboxError::Interrupted(_)) => {
            ExecutionFailure::Timeout
        }
        ExecutionError::Sandbox(SandboxError::Exit(exit)) => {
            ExecutionFailure::Exit(exit.to_string())
        }
        ExecutionError::Sandbox(SandboxError::Failed(error)) => ExecutionFailure::Failed(error),
        other @ (ExecutionError::Spawn { .. } | ExecutionError::TimeoutOutOfRange { .. }) => {
            ExecutionFailure::Unexpected(other.to_string())
        }
    }
}

fn module_problem(failure: ModuleLoadFailure) -> LoadProblem {
    match failure {
        ModuleLoadFailure::MissingFunction => LoadProblem::MissingFunction,
        ModuleLoadFailure::NotCallable => LoadProblem::NotCallable,
        ModuleLoadFailure::Runtime(error) => LoadProblem::Compile(error.to_string()),
    }
}

fn call_problem(source: ExecutionError<RuntimeFailure>) -> CallProblem {
    match execution_failure(source) {
        ExecutionFailure::Timeout => CallProblem::Timeout,
        ExecutionFailure::Exit(message)
        | ExecutionFailure::Unexpected(message)
        | ExecutionFailure::Failed(RuntimeFailure { message }) => CallProblem::Error(message),
    }
}
