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
use betterlite_error::envelope::ErrorObject;
use betterlite_error::envelope::ErrorResponse;
use betterlite_error::envelope::WireField;
use betterlite_guardrails::custom_code::runtime::CallOutput;
use betterlite_guardrails::custom_code::runtime::LoadFailure as ModuleLoadFailure;
use betterlite_guardrails::custom_code::runtime::RuntimeFailure;
use betterlite_guardrails::custom_code_execution::execution::error::ExecutionError;
use betterlite_guardrails::custom_code_execution::execution::error::ExecutionTimeoutError;
use betterlite_guardrails::custom_code_execution::execution::error::SandboxError;
use betterlite_guardrails::custom_code_execution::execution::error::SandboxExit;
use betterlite_guardrails::custom_code_execution::execution::eventloop::await_with_timeout;
use betterlite_guardrails::custom_code_execution::execution::offloop::call_off_loop_with_timeout;
use betterlite_guardrails::custom_code_execution::sandbox::DEFAULT_FILENAME;
use betterlite_guardrails::custom_code_execution::sandbox::compile_sandboxed;
use betterlite_guardrails::custom_code_execution::sandbox::globals::build_sandbox_globals;
use serde::Serialize;

use crate::auth::auth_identity::AuthContext;

use super::body::decode_test_request;
use super::error::CallFailure;
use super::error::LoadFailure;
use super::error::SandboxLoad;
use super::run::CallOutcome;
use super::run::CodeSandbox;
use super::run::EXECUTION_TIMEOUT;
use super::run::LoadOutcome;
use super::run::LoadedCode;
use super::run::SandboxHost;
use super::run::TestCall;
use super::run::run_test;

const ADMIN_REQUIRED: &str = "Admin access required to test custom code guardrails";
const MASTER_KEY_SUBJECT: &str = "master_key";
const UNAUTHENTICATED: &str = "the API key is not valid";

pub struct SandboxedCode {
    pub host: Arc<dyn SandboxHost>,
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
        return unauthenticated();
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

// LiteLLM's auth_error carries param "None" rather than null, which WireField::MISSING serializes
fn unauthenticated() -> Response {
    let body = ErrorResponse {
        error: ErrorObject {
            message: UNAUTHENTICATED.to_owned(),
            error_type: WireField::Text("auth_error".to_owned()),
            param: WireField::MISSING,
            code: WireField::Text("401".to_owned()),
        },
    };
    (StatusCode::UNAUTHORIZED, Json(body)).into_response()
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
                ExecutionFailure::Timeout(error) => LoadFailure::Timeout(error),
                ExecutionFailure::Exit(exit) => LoadFailure::Exit(exit),
                ExecutionFailure::Unexpected(error) => LoadFailure::Unexpected(error),
                ExecutionFailure::Failed(SandboxLoad::Compile(error)) => LoadFailure::Syntax(error),
                ExecutionFailure::Failed(SandboxLoad::Module(module)) => module_failure(module),
            }),
        }
    }

    async fn call(&self, code: LoadedCode, arguments: TestCall) -> CallOutcome {
        let started = Instant::now();
        let module = code.0;
        let inputs = GuardrailValue::Object(arguments.inputs);
        let request_data = GuardrailValue::Object(arguments.request_data);
        let input_type = arguments.input_type;
        let output = match call_off_loop_with_timeout(
            move || module.call(inputs, request_data, &input_type),
            EXECUTION_TIMEOUT,
            "test",
        )
        .await
        {
            Ok(output) => output,
            Err(error) => return CallOutcome::Failed(call_failure(error)),
        };
        let pending = match output {
            CallOutput::Value(value) => return CallOutcome::Result(value),
            CallOutput::Pending(pending) => pending,
        };
        let remaining = EXECUTION_TIMEOUT.saturating_sub(started.elapsed());
        match await_with_timeout(pending, remaining, "test").await {
            Ok(value) => CallOutcome::Result(value),
            Err(error) => CallOutcome::Failed(call_failure(error)),
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
    Timeout(ExecutionTimeoutError),
    Exit(SandboxExit),
    Failed(E),
    Unexpected(ExecutionError<E>),
}

fn execution_failure<E>(source: ExecutionError<E>) -> ExecutionFailure<E>
where
    E: StdError,
{
    match source {
        ExecutionError::Timeout(timeout) => ExecutionFailure::Timeout(timeout),
        ExecutionError::Sandbox(SandboxError::Interrupted(_)) => {
            ExecutionFailure::Timeout(ExecutionTimeoutError {
                timeout: EXECUTION_TIMEOUT,
            })
        }
        ExecutionError::Sandbox(SandboxError::Exit(exit)) => ExecutionFailure::Exit(exit),
        ExecutionError::Sandbox(SandboxError::Failed(error)) => ExecutionFailure::Failed(error),
        other @ (ExecutionError::Spawn { .. } | ExecutionError::TimeoutOutOfRange { .. }) => {
            ExecutionFailure::Unexpected(other)
        }
    }
}

fn module_failure(failure: ModuleLoadFailure) -> LoadFailure {
    match failure {
        ModuleLoadFailure::MissingFunction => LoadFailure::MissingFunction,
        ModuleLoadFailure::NotCallable => LoadFailure::NotCallable,
        other @ ModuleLoadFailure::Runtime(_) => LoadFailure::Load(other),
    }
}

fn call_failure(source: ExecutionError<RuntimeFailure>) -> CallFailure {
    match execution_failure(source) {
        ExecutionFailure::Timeout(timeout) => CallFailure::Timeout(timeout),
        ExecutionFailure::Exit(exit) => CallFailure::Exit(exit),
        ExecutionFailure::Unexpected(error) => CallFailure::Unexpected(error),
        ExecutionFailure::Failed(error) => CallFailure::Runtime(error),
    }
}
