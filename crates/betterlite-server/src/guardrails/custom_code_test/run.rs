use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::custom_code::runtime::CallOutput;
use betterlite_guardrails::custom_code::runtime::LoadFailure as ModuleLoadFailure;
use betterlite_guardrails::custom_code::runtime::RuntimeFailure;
use betterlite_guardrails::custom_code_execution::execution::error::ExecutionError;
use betterlite_guardrails::custom_code_execution::execution::error::SandboxError;
use betterlite_guardrails::custom_code_execution::sandbox::CompiledProgram;
use betterlite_guardrails::custom_code_execution::sandbox::globals::SandboxGlobals;

use super::body::TestCustomCodeRequest;
use super::error::CallFailure;
use super::error::LoadFailure;
use super::error::SandboxLoad;
use super::report::ErrorType;
use super::report::TestCustomCodeResponse;

pub const EXECUTION_TIMEOUT_SECONDS: u64 = 5;
pub const EXECUTION_TIMEOUT: Duration = Duration::from_secs(EXECUTION_TIMEOUT_SECONDS);

const TEST_MODEL: &str = "test-model";

// The route hands the function the input_type text itself, which B3803's two-value CallArguments cannot carry
pub trait SandboxModule: Send + Sync {
    fn call(
        &self,
        inputs: GuardrailValue,
        request_data: GuardrailValue,
        input_type: &str,
    ) -> Result<CallOutput, SandboxError<RuntimeFailure>>;
}

pub trait SandboxHost: Send + Sync {
    fn load(
        &self,
        program: &CompiledProgram,
        globals: &SandboxGlobals,
    ) -> Result<Box<dyn SandboxModule>, SandboxError<ModuleLoadFailure>>;
}

pub struct LoadedCode(pub Box<dyn SandboxModule>);

pub struct TestCall {
    pub inputs: BTreeMap<String, GuardrailValue>,
    pub request_data: BTreeMap<String, GuardrailValue>,
    pub input_type: String,
}

pub enum LoadOutcome {
    Loaded(LoadedCode),
    Failed(LoadFailure),
}

pub enum CallOutcome {
    Result(GuardrailValue),
    Failed(CallFailure),
}

#[async_trait]
pub trait CodeSandbox: Send + Sync {
    async fn load(&self, source: &str) -> LoadOutcome;
    async fn call(&self, code: LoadedCode, arguments: TestCall) -> CallOutcome;
}

pub async fn run_test(
    request: TestCustomCodeRequest,
    sandbox: &dyn CodeSandbox,
) -> TestCustomCodeResponse {
    let code = match sandbox.load(&request.custom_code).await {
        LoadOutcome::Loaded(code) => code,
        LoadOutcome::Failed(problem) => return load_failure_response(problem),
    };
    match sandbox.call(code, test_call(request)).await {
        CallOutcome::Result(value) => result_response(value),
        CallOutcome::Failed(CallFailure::Timeout(_)) => timeout_response(),
        CallOutcome::Failed(
            other @ (CallFailure::Runtime(_) | CallFailure::Exit(_) | CallFailure::Unexpected(_)),
        ) => TestCustomCodeResponse::failure(
            format!("Execution error: {other}"),
            ErrorType::Execution,
        ),
    }
}

fn test_call(request: TestCustomCodeRequest) -> TestCall {
    let TestCustomCodeRequest {
        mut test_input,
        input_type,
        request_data,
        ..
    } = request;
    test_input
        .entry("texts".to_owned())
        .or_insert_with(|| GuardrailValue::Array(Vec::new()));
    let given = request_data.unwrap_or_default();
    TestCall {
        inputs: test_input,
        request_data: request_data_with_defaults(&given),
        input_type,
    }
}

fn request_data_with_defaults(
    given: &BTreeMap<String, GuardrailValue>,
) -> BTreeMap<String, GuardrailValue> {
    let model = given
        .get("model")
        .cloned()
        .unwrap_or_else(|| GuardrailValue::Text(TEST_MODEL.to_owned()));
    let user_id = given
        .get("user_id")
        .cloned()
        .unwrap_or(GuardrailValue::Null);
    let team_id = given
        .get("team_id")
        .cloned()
        .unwrap_or(GuardrailValue::Null);
    let end_user_id = given
        .get("end_user_id")
        .cloned()
        .unwrap_or(GuardrailValue::Null);
    let metadata = given
        .get("metadata")
        .cloned()
        .unwrap_or_else(|| GuardrailValue::Object(BTreeMap::new()));
    BTreeMap::from([
        ("model".to_owned(), model),
        ("user_id".to_owned(), user_id),
        ("team_id".to_owned(), team_id),
        ("end_user_id".to_owned(), end_user_id),
        ("metadata".to_owned(), metadata),
    ])
}

fn load_failure_response(problem: LoadFailure) -> TestCustomCodeResponse {
    match problem {
        LoadFailure::Syntax(error) => TestCustomCodeResponse::failure(
            format!("Syntax error in custom code: {error}"),
            ErrorType::Compilation,
        ),
        other @ (LoadFailure::MissingFunction | LoadFailure::NotCallable) => {
            TestCustomCodeResponse::failure(other.to_string(), ErrorType::Compilation)
        }
        other @ (LoadFailure::Load(_) | LoadFailure::Exit(_)) => TestCustomCodeResponse::failure(
            format!("Failed to compile custom code: {other}"),
            ErrorType::Compilation,
        ),
        LoadFailure::Timeout(_) => timeout_response(),
        LoadFailure::Unexpected(error) => unexpected_response(&error),
    }
}

fn result_response(value: GuardrailValue) -> TestCustomCodeResponse {
    match value {
        GuardrailValue::Object(result) => TestCustomCodeResponse::success(result),
        other @ (GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Text(_)
        | GuardrailValue::Array(_)) => {
            let warning = format!(
                "Expected dict result, got {}. Treating as allow.",
                value_type_name(&other)
            );
            let mut result = BTreeMap::new();
            result.insert(
                "action".to_owned(),
                GuardrailValue::Text("allow".to_owned()),
            );
            result.insert("warning".to_owned(), GuardrailValue::Text(warning));
            TestCustomCodeResponse::success(result)
        }
    }
}

const fn value_type_name(value: &GuardrailValue) -> &'static str {
    match value {
        GuardrailValue::Null => "NoneType",
        GuardrailValue::Boolean(_) => "bool",
        GuardrailValue::SignedInteger(_) | GuardrailValue::UnsignedInteger(_) => "int",
        GuardrailValue::Decimal(_) => "float",
        GuardrailValue::Text(_) => "str",
        GuardrailValue::Array(_) => "list",
        GuardrailValue::Object(_) => "dict",
    }
}

fn timeout_response() -> TestCustomCodeResponse {
    TestCustomCodeResponse::failure(
        format!("Execution timeout: code took longer than {EXECUTION_TIMEOUT_SECONDS} seconds"),
        ErrorType::Execution,
    )
}

fn unexpected_response(error: &ExecutionError<SandboxLoad>) -> TestCustomCodeResponse {
    tracing::error!("Error testing custom code guardrail: {error}");
    TestCustomCodeResponse::failure(format!("Unexpected error: {error}"), ErrorType::Execution)
}
