use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::custom_code::runtime::LoadedModule;

use super::body::TestCustomCodeRequest;
use super::report::ErrorType;
use super::report::TestCustomCodeResponse;

pub const EXECUTION_TIMEOUT_SECONDS: u64 = 5;
pub const EXECUTION_TIMEOUT: Duration = Duration::from_secs(EXECUTION_TIMEOUT_SECONDS);

const MISSING_FUNCTION: &str = "Custom code must define an 'apply_guardrail' function. Expected signature: apply_guardrail(inputs, request_data, input_type)";
const NOT_CALLABLE: &str = "'apply_guardrail' must be a callable function";
const TEST_MODEL: &str = "test-model";

pub struct LoadedCode(pub Box<dyn LoadedModule>);

pub struct TestCall {
    pub inputs: BTreeMap<String, GuardrailValue>,
    pub request_data: BTreeMap<String, GuardrailValue>,
    pub input_type: String,
}

pub enum LoadOutcome {
    Loaded(LoadedCode),
    Failed(LoadProblem),
}

pub enum LoadProblem {
    Syntax(String),
    MissingFunction,
    NotCallable,
    Compile(String),
    Timeout,
    Unexpected(String),
}

pub enum CallOutcome {
    Result(GuardrailValue),
    Failed(CallProblem),
}

pub enum CallProblem {
    Error(String),
    Timeout,
    Unexpected(String),
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
        CallOutcome::Failed(CallProblem::Error(error)) => TestCustomCodeResponse::failure(
            format!("Execution error: {error}"),
            ErrorType::Execution,
        ),
        CallOutcome::Failed(CallProblem::Timeout) => timeout_response(),
        CallOutcome::Failed(CallProblem::Unexpected(error)) => unexpected_response(&error),
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

fn load_failure_response(problem: LoadProblem) -> TestCustomCodeResponse {
    match problem {
        LoadProblem::Syntax(error) => TestCustomCodeResponse::failure(
            format!("Syntax error in custom code: {error}"),
            ErrorType::Compilation,
        ),
        LoadProblem::MissingFunction => {
            TestCustomCodeResponse::failure(MISSING_FUNCTION.to_owned(), ErrorType::Compilation)
        }
        LoadProblem::NotCallable => {
            TestCustomCodeResponse::failure(NOT_CALLABLE.to_owned(), ErrorType::Compilation)
        }
        LoadProblem::Compile(error) => TestCustomCodeResponse::failure(
            format!("Failed to compile custom code: {error}"),
            ErrorType::Compilation,
        ),
        LoadProblem::Timeout => timeout_response(),
        LoadProblem::Unexpected(error) => unexpected_response(&error),
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

fn unexpected_response(error: &str) -> TestCustomCodeResponse {
    tracing::error!("Error testing custom code guardrail: {error}");
    TestCustomCodeResponse::failure(format!("Unexpected error: {error}"), ErrorType::Execution)
}
