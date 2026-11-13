#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::future::pending;
use std::io::Error as IoError;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::thread::park_timeout;
use std::thread::yield_now;
use std::time::Duration;
use std::time::Instant;

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::body::to_bytes;
use axum::http::Request;
use axum::http::StatusCode;
use axum::http::header;
use betterlite_auth::auth_identity::records::CredentialRef;
use betterlite_auth::auth_identity::records::NetworkContext;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::records::SourceKey;
use betterlite_auth::auth_identity::roles::AuthMethod;
use betterlite_auth::auth_identity::roles::PrincipalType;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::custom_code::runtime::CallOutput;
use betterlite_guardrails::custom_code::runtime::LoadFailure as ModuleLoadFailure;
use betterlite_guardrails::custom_code::runtime::RuntimeFailure;
use betterlite_guardrails::custom_code_execution::deadline;
use betterlite_guardrails::custom_code_execution::execution::error::ExecutionError;
use betterlite_guardrails::custom_code_execution::execution::error::SandboxError;
use betterlite_guardrails::custom_code_execution::sandbox::CompiledProgram;
use betterlite_guardrails::custom_code_execution::sandbox::globals::SandboxGlobals;
use rstest::rstest;
use tokio::time::sleep;
use tower::ServiceExt as _;

use crate::auth::auth_identity::AuthContext;

use super::body::TestCustomCodeRequest;
use super::body::decode_test_request;
use super::error::CallFailure;
use super::error::LoadFailure;
use super::report::ErrorType;
use super::report::TestCustomCodeResponse;
use super::route::SandboxedCode;
use super::route::routes;
use super::run::CallOutcome;
use super::run::CodeSandbox;
use super::run::LoadOutcome;
use super::run::LoadedCode;
use super::run::SandboxHost;
use super::run::SandboxModule;
use super::run::TestCall;
use super::run::run_test;

const ALLOW_SOURCE: &str =
    "def apply_guardrail(inputs, request_data, input_type):\n    return allow()\n";
const ADMIN: &str = "key-admin";
const VIEWER: &str = "key-viewer";
const INTERNAL: &str = "key-internal";
const MASTER: &str = "master_key";
const ADMIN_REQUIRED: &str = "Admin access required to test custom code guardrails";

#[derive(Clone)]
enum Answer {
    Value(GuardrailValue),
    Pending(GuardrailValue),
    // Three seconds before the coroutine and three inside it, so only the shared deadline cuts it at the timeout
    Slow(GuardrailValue),
    PendingForever,
    Error(String),
}

const SLOW_STEP: Duration = Duration::from_secs(3);

enum HostReply {
    Answer(Answer),
    MissingFunction,
    NotCallable,
    Runtime(&'static str),
    Stuck,
}

#[derive(Clone)]
struct RecordedCall {
    inputs: GuardrailValue,
    request_data: GuardrailValue,
    input_type: String,
}

struct FakeModule {
    answer: Answer,
    seen: Arc<Mutex<Vec<RecordedCall>>>,
}

impl SandboxModule for FakeModule {
    fn call(
        &self,
        inputs: GuardrailValue,
        request_data: GuardrailValue,
        input_type: &str,
    ) -> Result<CallOutput, SandboxError<RuntimeFailure>> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedCall {
                inputs,
                request_data,
                input_type: input_type.to_owned(),
            });
        match &self.answer {
            Answer::Value(value) => Ok(CallOutput::Value(value.clone())),
            Answer::Pending(value) => {
                let settled = value.clone();
                Ok(CallOutput::Pending(Box::pin(async move {
                    Ok::<GuardrailValue, SandboxError<RuntimeFailure>>(settled)
                })))
            }
            Answer::Slow(value) => {
                let started = Instant::now();
                while started.elapsed() < SLOW_STEP {
                    park_timeout(SLOW_STEP.saturating_sub(started.elapsed()));
                }
                let settled = value.clone();
                Ok(CallOutput::Pending(Box::pin(async move {
                    sleep(SLOW_STEP).await;
                    Ok::<GuardrailValue, SandboxError<RuntimeFailure>>(settled)
                })))
            }
            Answer::PendingForever => Ok(CallOutput::Pending(Box::pin(pending::<
                Result<GuardrailValue, SandboxError<RuntimeFailure>>,
            >()))),
            Answer::Error(message) => Err(SandboxError::Failed(RuntimeFailure {
                message: message.clone(),
            })),
        }
    }
}

struct FakeHost {
    loads: AtomicUsize,
    sources: Mutex<Vec<String>>,
    globals: Mutex<Option<SandboxGlobals>>,
    seen: Arc<Mutex<Vec<RecordedCall>>>,
    reply: HostReply,
}

impl SandboxHost for FakeHost {
    fn load(
        &self,
        program: &CompiledProgram,
        globals: &SandboxGlobals,
    ) -> Result<Box<dyn SandboxModule>, SandboxError<ModuleLoadFailure>> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        self.sources
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(program.source.clone());
        *self.globals.lock().unwrap_or_else(PoisonError::into_inner) = Some(globals.clone());
        match &self.reply {
            HostReply::Answer(answer) => Ok(Box::new(FakeModule {
                answer: answer.clone(),
                seen: Arc::clone(&self.seen),
            })),
            HostReply::MissingFunction => {
                Err(SandboxError::Failed(ModuleLoadFailure::MissingFunction))
            }
            HostReply::NotCallable => Err(SandboxError::Failed(ModuleLoadFailure::NotCallable)),
            HostReply::Runtime(message) => Err(SandboxError::Failed(ModuleLoadFailure::Runtime(
                RuntimeFailure {
                    message: (*message).to_owned(),
                },
            ))),
            HostReply::Stuck => loop {
                if let Err(interrupted) = deadline::budget_ok() {
                    return Err(SandboxError::Interrupted(interrupted));
                }
                yield_now();
            },
        }
    }
}

fn host(reply: HostReply) -> Arc<FakeHost> {
    Arc::new(FakeHost {
        loads: AtomicUsize::new(0),
        sources: Mutex::new(Vec::new()),
        globals: Mutex::new(None),
        seen: Arc::new(Mutex::new(Vec::new())),
        reply,
    })
}

fn sandbox(host: &Arc<FakeHost>) -> Arc<SandboxedCode> {
    let cloned = Arc::clone(host);
    let dynamic: Arc<dyn SandboxHost> = cloned;
    Arc::new(SandboxedCode { host: dynamic })
}

struct Scripted {
    load: Mutex<Option<LoadOutcome>>,
    call: Mutex<Option<CallOutcome>>,
}

fn scripted_sandbox(load: LoadOutcome, call: CallOutcome) -> Arc<Scripted> {
    Arc::new(Scripted {
        load: Mutex::new(Some(load)),
        call: Mutex::new(Some(call)),
    })
}

#[async_trait]
impl CodeSandbox for Scripted {
    async fn load(&self, _source: &str) -> LoadOutcome {
        self.load.lock().unwrap().take().unwrap_or_else(|| {
            LoadOutcome::Failed(LoadFailure::Unexpected(ExecutionError::Spawn {
                source: IoError::other("no scripted load"),
            }))
        })
    }

    async fn call(&self, _code: LoadedCode, _arguments: TestCall) -> CallOutcome {
        self.call.lock().unwrap().take().unwrap_or_else(|| {
            CallOutcome::Failed(CallFailure::Unexpected(ExecutionError::Spawn {
                source: IoError::other("no scripted call"),
            }))
        })
    }
}

fn last_call(fake: &Arc<FakeHost>) -> RecordedCall {
    fake.seen.lock().unwrap().last().unwrap().clone()
}

fn caller(subject: &str, roles: Vec<Role>) -> AuthContext {
    AuthContext {
        principal: Principal {
            principal_type: PrincipalType::ServiceAccount,
            subject: subject.to_owned(),
            issuer: None,
            audience: Vec::new(),
            user: None,
            organization: None,
            teams: Vec::new(),
            project: None,
            end_user: None,
            roles,
            scopes: Vec::new(),
            auth_method: AuthMethod::ApiKey,
            credential_ref: CredentialRef::default(),
            network: NetworkContext::default(),
            source_key: SourceKey::default(),
        },
    }
}

fn request(source: &str) -> TestCustomCodeRequest {
    TestCustomCodeRequest {
        custom_code: source.to_owned(),
        test_input: BTreeMap::from([(
            "texts".to_owned(),
            GuardrailValue::Array(vec![GuardrailValue::Text("hello".to_owned())]),
        )]),
        input_type: "request".to_owned(),
        request_data: None,
    }
}

async fn post(application: Router, auth: Option<AuthContext>, body: &str) -> (StatusCode, String) {
    let builder = Request::builder()
        .method("POST")
        .uri("/guardrails/test_custom_code")
        .header(header::CONTENT_TYPE, "application/json");
    let mut request = builder.body(Body::from(body.to_owned())).unwrap();
    if let Some(context) = auth {
        request.extensions_mut().insert(context);
    }
    let response = application.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

fn object(text: &str) -> BTreeMap<String, GuardrailValue> {
    let GuardrailValue::Object(fields) = serde_json::from_str(text).unwrap() else {
        panic!("object fixture");
    };
    fields
}

fn allow_answer() -> HostReply {
    HostReply::Answer(Answer::Value(GuardrailValue::Object(object(
        r#"{"action":"allow"}"#,
    ))))
}

fn quoted(text: &str) -> String {
    serde_json::to_string(text).unwrap()
}

fn body_with_inputs() -> String {
    format!(
        r#"{{"custom_code":{},"test_input":{{"texts":["hello"]}}}}"#,
        quoted(ALLOW_SOURCE)
    )
}

#[tokio::test]
async fn access_proxy_admin_reaches_the_sandbox() {
    let fake = host(allow_answer());
    let (status, body) = post(
        routes(sandbox(&fake)),
        Some(caller(ADMIN, vec![Role::PlatformAdmin])),
        &body_with_inputs(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        r#"{"success":true,"result":{"action":"allow"},"error":null,"error_type":null}"#
    );
    assert_eq!(fake.loads.load(Ordering::SeqCst), 1);
    assert!(
        fake.sources
            .lock()
            .unwrap()
            .first()
            .unwrap()
            .contains("apply_guardrail"),
        "the sandbox receives the compiled source"
    );
}

#[tokio::test]
async fn access_master_key_is_a_proxy_admin() {
    let fake = host(allow_answer());
    let (status, _body) = post(
        routes(sandbox(&fake)),
        Some(caller(MASTER, Vec::new())),
        &body_with_inputs(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn access_proxy_admin_viewer_is_forbidden() {
    let fake = host(allow_answer());
    let (status, body) = post(
        routes(sandbox(&fake)),
        Some(caller(VIEWER, vec![Role::PlatformViewer])),
        &body_with_inputs(),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body, format!(r#"{{"detail":"{ADMIN_REQUIRED}"}}"#));
    assert_eq!(fake.loads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn access_internal_user_is_forbidden() {
    let fake = host(allow_answer());
    let (status, body) = post(
        routes(sandbox(&fake)),
        Some(caller(INTERNAL, Vec::new())),
        &body_with_inputs(),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body, format!(r#"{{"detail":"{ADMIN_REQUIRED}"}}"#));
}

#[tokio::test]
async fn access_without_a_key_is_unauthorized() {
    let fake = host(allow_answer());
    let (status, body) = post(routes(sandbox(&fake)), None, &body_with_inputs()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        body,
        r#"{"error":{"message":"the API key is not valid","type":"auth_error","param":"None","code":"401"}}"#
    );
    assert_eq!(fake.loads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn body_defaults_are_applied() {
    let fake = host(allow_answer());
    let (status, _body) = post(
        routes(sandbox(&fake)),
        Some(caller(ADMIN, vec![Role::PlatformAdmin])),
        &body_with_inputs(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let arguments = last_call(&fake);
    assert_eq!(arguments.input_type, "request");
    let GuardrailValue::Object(request_data) = &arguments.request_data else {
        panic!("request data object");
    };
    assert_eq!(request_data.len(), 5);
    assert_eq!(
        request_data["model"],
        GuardrailValue::Text("test-model".to_owned())
    );
    assert_eq!(request_data["user_id"], GuardrailValue::Null);
    assert_eq!(request_data["team_id"], GuardrailValue::Null);
    assert_eq!(request_data["end_user_id"], GuardrailValue::Null);
    assert_eq!(
        request_data["metadata"],
        GuardrailValue::Object(BTreeMap::new())
    );
}

#[tokio::test]
async fn body_unknown_keys_are_ignored() {
    let fake = host(allow_answer());
    let body = format!(
        r#"{{"custom_code":{},"test_input":{{}},"unknown":1,"nested":{{"a":true}}}}"#,
        quoted(ALLOW_SOURCE)
    );
    let (status, _body) = post(
        routes(sandbox(&fake)),
        Some(caller(ADMIN, vec![Role::PlatformAdmin])),
        &body,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn body_missing_custom_code_is_422() {
    let fake = host(allow_answer());
    let (status, body) = post(
        routes(sandbox(&fake)),
        Some(caller(ADMIN, vec![Role::PlatformAdmin])),
        r#"{"test_input":{}}"#,
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body,
        r#"{"detail":[{"type":"missing","loc":["body","custom_code"],"msg":"Field required"}]}"#
    );
}

#[tokio::test]
async fn body_missing_test_input_is_422() {
    let fake = host(allow_answer());
    let (status, body) = post(
        routes(sandbox(&fake)),
        Some(caller(ADMIN, vec![Role::PlatformAdmin])),
        r#"{"custom_code":"x"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body,
        r#"{"detail":[{"type":"missing","loc":["body","test_input"],"msg":"Field required"}]}"#
    );
}

#[tokio::test]
async fn body_invalid_json_is_422() {
    let fake = host(allow_answer());
    let (status, body) = post(
        routes(sandbox(&fake)),
        Some(caller(ADMIN, vec![Role::PlatformAdmin])),
        "{",
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body,
        r#"{"detail":[{"type":"json_invalid","loc":["body"],"msg":"JSON decode error"}]}"#
    );
}

#[tokio::test]
async fn body_trailing_data_is_422() {
    let fake = host(allow_answer());
    let (status, body) = post(
        routes(sandbox(&fake)),
        Some(caller(ADMIN, vec![Role::PlatformAdmin])),
        r#"{"custom_code":"x","test_input":{}} trailing"#,
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body,
        r#"{"detail":[{"type":"json_invalid","loc":["body"],"msg":"JSON decode error"}]}"#
    );
}

#[tokio::test]
async fn body_is_checked_before_the_role_check() {
    let fake = host(allow_answer());
    let (status, _body) = post(
        routes(sandbox(&fake)),
        Some(caller(VIEWER, vec![Role::PlatformViewer])),
        r#"{"test_input":{}}"#,
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(fake.loads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn body_json_numbers_keep_their_kind() {
    let fake = host(allow_answer());
    let body = format!(
        r#"{{"custom_code":{},"test_input":{{"count":3,"negative":-2,"ratio":1.25,"big":18446744073709551615}}}}"#,
        quoted(ALLOW_SOURCE)
    );
    let (status, _body) = post(
        routes(sandbox(&fake)),
        Some(caller(ADMIN, vec![Role::PlatformAdmin])),
        &body,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let arguments = last_call(&fake);
    let GuardrailValue::Object(inputs) = &arguments.inputs else {
        panic!("inputs object");
    };
    assert_eq!(inputs["count"], GuardrailValue::SignedInteger(3));
    assert_eq!(inputs["negative"], GuardrailValue::SignedInteger(-2));
    assert_eq!(inputs["ratio"], GuardrailValue::Decimal(1.25));
    assert_eq!(inputs["big"], GuardrailValue::UnsignedInteger(u64::MAX));
}

#[tokio::test]
async fn body_request_data_null_takes_the_default() {
    let fake = host(allow_answer());
    let body = format!(
        r#"{{"custom_code":{},"test_input":{{}},"request_data":null}}"#,
        quoted(ALLOW_SOURCE)
    );
    let (status, _body) = post(
        routes(sandbox(&fake)),
        Some(caller(ADMIN, vec![Role::PlatformAdmin])),
        &body,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let arguments = last_call(&fake);
    let GuardrailValue::Object(request_data) = &arguments.request_data else {
        panic!("request data object");
    };
    assert_eq!(
        request_data["model"],
        GuardrailValue::Text("test-model".to_owned())
    );
}

#[tokio::test]
async fn body_nesting_past_the_limit_is_422() {
    let fake = host(allow_answer());
    let deep = format!("{}null{}", "[".repeat(70), "]".repeat(70));
    let body = format!(
        r#"{{"custom_code":{},"test_input":{{"deep":{deep}}}}}"#,
        quoted(ALLOW_SOURCE)
    );
    let (status, text) = post(
        routes(sandbox(&fake)),
        Some(caller(ADMIN, vec![Role::PlatformAdmin])),
        &body,
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(text.contains("nests deeper than 64 levels"), "{text}");
    assert_eq!(fake.loads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn compile_syntax_error_is_reported() {
    let fake = host(HostReply::MissingFunction);
    let response = run_test(request("def apply_guardrail(:\n"), sandbox(&fake).as_ref()).await;
    assert!(!response.success);
    assert_eq!(response.error_type, Some(ErrorType::Compilation));
    let error = response.error.unwrap();
    assert!(error.starts_with("Syntax error in custom code:"), "{error}");
    assert_eq!(fake.loads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn compile_import_is_refused() {
    let fake = host(HostReply::MissingFunction);
    let response = run_test(request("import os\n"), sandbox(&fake).as_ref()).await;
    assert_eq!(response.error_type, Some(ErrorType::Compilation));
    let error = response.error.unwrap();
    assert!(error.starts_with("Syntax error in custom code:"), "{error}");
    assert_eq!(fake.loads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn compile_missing_function_is_reported() {
    let fake = host(HostReply::MissingFunction);
    let response = run_test(
        request("def check():\n    return 1\n"),
        sandbox(&fake).as_ref(),
    )
    .await;
    assert!(!response.success);
    assert_eq!(response.error_type, Some(ErrorType::Compilation));
    assert_eq!(
        response.error.unwrap(),
        "Custom code must define an 'apply_guardrail' function. Expected signature: apply_guardrail(inputs, request_data, input_type)"
    );
}

#[tokio::test]
async fn compile_not_callable_is_reported() {
    let fake = host(HostReply::NotCallable);
    let response = run_test(request(ALLOW_SOURCE), sandbox(&fake).as_ref()).await;
    assert_eq!(response.error_type, Some(ErrorType::Compilation));
    assert_eq!(
        response.error.unwrap(),
        "'apply_guardrail' must be a callable function"
    );
}

#[tokio::test]
async fn compile_hands_the_primitives_and_guards_to_the_host() {
    let fake = host(allow_answer());
    let response = run_test(request(ALLOW_SOURCE), sandbox(&fake).as_ref()).await;
    assert!(response.success);
    let globals = fake.globals.lock().unwrap().clone().unwrap();
    assert!(globals.entries.contains_key("allow"));
    assert!(globals.entries.contains_key("regex_match"));
    assert!(globals.entries.contains_key("_budget_ok_"));
    assert!(globals.entries.contains_key("__builtins__"));
}

#[tokio::test]
async fn compile_load_failure_is_reported() {
    let fake = host(HostReply::Runtime("name 'allow' is not defined"));
    let response = run_test(request(ALLOW_SOURCE), sandbox(&fake).as_ref()).await;
    assert_eq!(response.error_type, Some(ErrorType::Compilation));
    assert_eq!(
        response.error.unwrap(),
        "Failed to compile custom code: name 'allow' is not defined"
    );
}

#[rstest]
#[case::allow(r#"{"action":"allow"}"#)]
#[case::block(r#"{"action":"block","reason":"ssn detected"}"#)]
#[case::modify(r#"{"action":"modify","texts":["redacted"]}"#)]
#[tokio::test]
async fn run_object_result_is_returned(#[case] result: &str) {
    let expected = object(result);
    let fake = host(HostReply::Answer(Answer::Value(GuardrailValue::Object(
        expected.clone(),
    ))));
    let response = run_test(request(ALLOW_SOURCE), sandbox(&fake).as_ref()).await;
    assert!(response.success);
    assert_eq!(response.result, Some(expected));
    assert_eq!(response.error, None);
    assert_eq!(response.error_type, None);
}

#[tokio::test]
async fn run_async_function_result_is_awaited() {
    let expected = object(r#"{"action":"block","reason":"async"}"#);
    let fake = host(HostReply::Answer(Answer::Pending(GuardrailValue::Object(
        expected.clone(),
    ))));
    let response = run_test(request(ALLOW_SOURCE), sandbox(&fake).as_ref()).await;
    assert!(response.success);
    assert_eq!(response.result, Some(expected));
}

#[tokio::test]
async fn run_input_type_text_is_passed_on() {
    let fake = host(allow_answer());
    let mut given = request(ALLOW_SOURCE);
    given.input_type = "foo".to_owned();
    let response = run_test(given, sandbox(&fake).as_ref()).await;
    assert!(response.success);
    assert_eq!(last_call(&fake).input_type, "foo");
}

#[tokio::test]
async fn run_response_input_type_reaches_the_sandbox_call() {
    let fake = host(allow_answer());
    let mut given = request(ALLOW_SOURCE);
    given.input_type = "response".to_owned();
    let response = run_test(given, sandbox(&fake).as_ref()).await;
    assert!(response.success);
    assert_eq!(last_call(&fake).input_type, "response");
}

#[tokio::test]
async fn run_request_data_has_exactly_five_keys() {
    let fake = host(allow_answer());
    let mut given = request(ALLOW_SOURCE);
    given.request_data = Some(BTreeMap::from([
        (
            "model".to_owned(),
            GuardrailValue::Text("gpt-4o".to_owned()),
        ),
        ("user_id".to_owned(), GuardrailValue::Text("u1".to_owned())),
        (
            "api_key".to_owned(),
            GuardrailValue::Text("sk-secret".to_owned()),
        ),
    ]));
    let response = run_test(given, sandbox(&fake).as_ref()).await;
    assert!(response.success);
    let arguments = last_call(&fake);
    let GuardrailValue::Object(request_data) = &arguments.request_data else {
        panic!("request data object");
    };
    assert_eq!(request_data.len(), 5);
    assert_eq!(
        request_data["model"],
        GuardrailValue::Text("gpt-4o".to_owned())
    );
    assert_eq!(
        request_data["user_id"],
        GuardrailValue::Text("u1".to_owned())
    );
    assert_eq!(request_data["team_id"], GuardrailValue::Null);
    assert!(!request_data.contains_key("api_key"));
}

#[tokio::test]
async fn run_texts_default_is_added() {
    let fake = host(allow_answer());
    let mut given = request(ALLOW_SOURCE);
    given.test_input = BTreeMap::new();
    let response = run_test(given, sandbox(&fake).as_ref()).await;
    assert!(response.success);
    let arguments = last_call(&fake);
    let GuardrailValue::Object(inputs) = &arguments.inputs else {
        panic!("inputs object");
    };
    assert_eq!(inputs["texts"], GuardrailValue::Array(Vec::new()));
}

#[tokio::test]
async fn run_existing_texts_are_kept() {
    let fake = host(allow_answer());
    let response = run_test(request(ALLOW_SOURCE), sandbox(&fake).as_ref()).await;
    assert!(response.success);
    let arguments = last_call(&fake);
    let GuardrailValue::Object(inputs) = &arguments.inputs else {
        panic!("inputs object");
    };
    assert_eq!(
        inputs["texts"],
        GuardrailValue::Array(vec![GuardrailValue::Text("hello".to_owned())])
    );
}

#[tokio::test]
async fn run_explicitly_null_texts_are_kept() {
    let fake = host(allow_answer());
    let mut given = request(ALLOW_SOURCE);
    given.test_input = BTreeMap::from([("texts".to_owned(), GuardrailValue::Null)]);
    let response = run_test(given, sandbox(&fake).as_ref()).await;
    assert!(response.success);
    let arguments = last_call(&fake);
    let GuardrailValue::Object(inputs) = &arguments.inputs else {
        panic!("inputs object");
    };
    assert_eq!(inputs["texts"], GuardrailValue::Null);
}

#[tokio::test]
async fn run_request_data_explicit_values_are_kept() {
    let fake = host(allow_answer());
    let mut given = request(ALLOW_SOURCE);
    given.request_data = Some(BTreeMap::from([
        ("model".to_owned(), GuardrailValue::Null),
        ("metadata".to_owned(), GuardrailValue::Null),
    ]));
    let response = run_test(given, sandbox(&fake).as_ref()).await;
    assert!(response.success);
    let arguments = last_call(&fake);
    let GuardrailValue::Object(request_data) = &arguments.request_data else {
        panic!("request data object");
    };
    assert_eq!(request_data.len(), 5);
    assert_eq!(request_data["model"], GuardrailValue::Null);
    assert_eq!(request_data["metadata"], GuardrailValue::Null);
}

#[tokio::test]
async fn compile_failure_answers_200_over_http() {
    let fake = host(allow_answer());
    let (status, text) = post(
        routes(sandbox(&fake)),
        Some(caller(ADMIN, vec![Role::PlatformAdmin])),
        r#"{"custom_code":"def apply_guardrail(:","test_input":{}}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let response: TestCustomCodeResponse = serde_json::from_str(&text).unwrap();
    assert!(!response.success);
    assert_eq!(response.result, None);
    assert_eq!(response.error_type, Some(ErrorType::Compilation));
    assert!(
        response
            .error
            .unwrap()
            .starts_with("Syntax error in custom code:")
    );
}

#[tokio::test]
async fn run_runtime_error_is_execution_error() {
    let fake = host(HostReply::Answer(Answer::Error("boom".to_owned())));
    let response = run_test(request(ALLOW_SOURCE), sandbox(&fake).as_ref()).await;
    assert!(!response.success);
    assert_eq!(response.error_type, Some(ErrorType::Execution));
    assert_eq!(response.error.unwrap(), "Execution error: boom");
}

#[rstest]
#[case::none(GuardrailValue::Null, "NoneType")]
#[case::bool(GuardrailValue::Boolean(true), "bool")]
#[case::int(GuardrailValue::SignedInteger(7), "int")]
#[case::unsigned(GuardrailValue::UnsignedInteger(7), "int")]
#[case::float(GuardrailValue::Decimal(1.5), "float")]
#[case::text(GuardrailValue::Text("x".to_owned()), "str")]
#[case::list(GuardrailValue::Array(Vec::new()), "list")]
#[tokio::test]
async fn result_non_object_becomes_allow_with_warning(
    #[case] value: GuardrailValue,
    #[case] name: &str,
) {
    let fake = host(HostReply::Answer(Answer::Value(value)));
    let response = run_test(request(ALLOW_SOURCE), sandbox(&fake).as_ref()).await;
    assert!(response.success);
    assert_eq!(response.error, None);
    assert_eq!(response.error_type, None);
    assert_eq!(
        response.result,
        Some(BTreeMap::from([
            (
                "action".to_owned(),
                GuardrailValue::Text("allow".to_owned())
            ),
            (
                "warning".to_owned(),
                GuardrailValue::Text(format!(
                    "Expected dict result, got {name}. Treating as allow."
                ))
            ),
        ]))
    );
}

#[tokio::test]
async fn result_load_past_five_seconds_times_out() {
    let fake = host(HostReply::Stuck);
    let response = run_test(request(ALLOW_SOURCE), sandbox(&fake).as_ref()).await;
    assert!(!response.success);
    assert_eq!(response.error_type, Some(ErrorType::Execution));
    assert_eq!(
        response.error.unwrap(),
        "Execution timeout: code took longer than 5 seconds"
    );
}

#[tokio::test]
async fn result_run_past_five_seconds_times_out() {
    let fake = host(HostReply::Answer(Answer::PendingForever));
    let response = run_test(request(ALLOW_SOURCE), sandbox(&fake).as_ref()).await;
    assert!(!response.success);
    assert_eq!(response.error_type, Some(ErrorType::Execution));
    assert_eq!(
        response.error.unwrap(),
        "Execution timeout: code took longer than 5 seconds"
    );
}

#[tokio::test]
async fn result_async_result_shares_the_deadline() {
    let fake = host(HostReply::Answer(Answer::Slow(GuardrailValue::Object(
        object(r#"{"action":"allow"}"#),
    ))));
    let response = run_test(request(ALLOW_SOURCE), sandbox(&fake).as_ref()).await;
    assert!(!response.success);
    assert_eq!(response.error_type, Some(ErrorType::Execution));
    assert_eq!(
        response.error.unwrap(),
        "Execution timeout: code took longer than 5 seconds"
    );
}

#[tokio::test]
async fn result_unexpected_error_is_reported() {
    let scripted = scripted_sandbox(
        LoadOutcome::Failed(LoadFailure::Unexpected(ExecutionError::Spawn {
            source: IoError::other("worker unavailable"),
        })),
        CallOutcome::Result(GuardrailValue::Null),
    );
    let response = run_test(request(ALLOW_SOURCE), scripted.as_ref()).await;
    assert!(!response.success);
    assert_eq!(response.error_type, Some(ErrorType::Execution));
    assert_eq!(
        response.error.unwrap(),
        "Unexpected error: could not start the guardrail worker: worker unavailable"
    );
}

#[rstest]
fn replays_the_custom_code_test_body_corpus(
    #[files("fuzz/corpus/custom_code_test_body/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    drop(decode_test_request(&bytes));
}

#[test]
fn report_keeps_every_key() {
    let response = TestCustomCodeResponse::failure("x".to_owned(), ErrorType::Compilation);
    assert_eq!(
        serde_json::to_string(&response).unwrap(),
        r#"{"success":false,"result":null,"error":"x","error_type":"compilation"}"#
    );
}
