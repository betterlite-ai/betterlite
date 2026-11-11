use std::sync::Arc;
use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_core::routes::CallType;
use betterlite_error::error::ErrorClass;
use betterlite_error::error::ErrorContext;
use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::Marker;
use super::support::as_hook;
use super::support::request;
use crate::call_setup::RequestMap;
use crate::call_setup::correlation::CallOutcome;
use crate::call_setup::correlation::ChatStream;
use crate::call_setup::hooks::DeploymentHook;
use crate::call_setup::hooks::Guardrail;
use crate::call_setup::hooks::HookError;
use crate::call_setup::hooks::failure_deployment_hook;
use crate::call_setup::hooks::hook_call_type;
use crate::call_setup::hooks::pre_call_deployment_hook;
use crate::call_setup::hooks::run_success_hook_on_stream;
use crate::call_setup::hooks::success_deployment_hook;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Fault {
    #[default]
    Silent,
    Pre,
    Success,
    FailureHook,
}

struct Hook {
    label: String,
    guardrail: Option<Guardrail>,
    pre_insert: Option<(String, Json)>,
    success_replacement: Option<Json>,
    fault: Fault,
    pre_calls: Mutex<usize>,
    success_calls: Mutex<usize>,
    saw_targets: Mutex<Vec<bool>>,
    depths: Mutex<Vec<Option<i64>>>,
    saw_model: Mutex<Vec<bool>>,
}

impl Hook {
    fn silent(label: &str) -> Self {
        Self {
            label: label.to_owned(),
            guardrail: None,
            pre_insert: None,
            success_replacement: None,
            fault: Fault::Silent,
            pre_calls: Mutex::new(0),
            success_calls: Mutex::new(0),
            saw_targets: Mutex::new(Vec::new()),
            depths: Mutex::new(Vec::new()),
            saw_model: Mutex::new(Vec::new()),
        }
    }

    fn fail_if(&self, kind: Fault, message: &str) -> Result<(), HookError> {
        if self.fault == kind {
            return Err(HookError::Failed {
                hook: self.label.clone(),
                message: message.to_owned(),
            });
        }
        Ok(())
    }
}

#[async_trait]
impl CallbackLogger for Hook {
    fn callback_name(&self) -> &str {
        &self.label
    }
}

#[async_trait]
impl DeploymentHook for Hook {
    fn guardrail(&self) -> Option<Guardrail> {
        self.guardrail.clone()
    }

    async fn pre_call_deployment_hook(
        &self,
        request: &mut RequestMap,
        _call_type: Option<CallType>,
    ) -> Result<Option<RequestMap>, HookError> {
        if let Some((key, value)) = &self.pre_insert {
            request.insert(key.clone(), value.clone());
        }
        self.fail_if(Fault::Pre, "pre-call hook failed")?;
        bump(&self.pre_calls);
        Ok(None)
    }

    async fn success_deployment_hook(
        &self,
        _request_data: &RequestMap,
        _response: &Json,
        _call_type: Option<CallType>,
    ) -> Result<Option<Json>, HookError> {
        self.fail_if(Fault::Success, "success hook failed")?;
        bump(&self.success_calls);
        let replacement = self.success_replacement.clone();
        Ok(replacement)
    }

    async fn failure_deployment_hook(
        &self,
        request_data: &RequestMap,
        _error: &ErrorClass,
        _call_type: Option<CallType>,
        fallback_depth: Option<i64>,
    ) -> Result<(), HookError> {
        self.saw_targets
            .lock()
            .expect("the target log is free")
            .push(request_data.contains_key("attempted_targets"));
        self.depths
            .lock()
            .expect("the depth log is free")
            .push(fallback_depth);
        self.saw_model
            .lock()
            .expect("the model log is free")
            .push(request_data.contains_key("model"));
        self.fail_if(Fault::FailureHook, "failure hook failed")
    }
}

fn failure() -> ErrorClass {
    ErrorClass::Unclassified(Box::new(ErrorContext {
        message: String::from("the call failed"),
        model: None,
        provider: None,
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }))
}

fn arc(hook: Hook) -> Arc<dyn DeploymentHook> {
    Arc::new(hook)
}

fn bump(counter: &Mutex<usize>) {
    let mut calls = counter.lock().expect("the call counter is free");
    *calls = calls.saturating_add(1);
}

#[rstest]
#[case::known("completion", Some(CallType::Completion))]
#[case::async_call("acompletion", Some(CallType::Acompletion))]
#[case::unknown("some_new_call", None)]
fn deployment_hook_call_type_parses(#[case] name: &str, #[case] expected: Option<CallType>) {
    assert_eq!(hook_call_type(name), expected);
}

#[tokio::test]
async fn deployment_hook_pre_call_lets_a_later_hook_see_the_change() {
    let mut spec = Hook::silent("first");
    spec.pre_insert = Some((String::from("marker"), Json::Bool(true)));
    let first = Arc::new(spec);
    let second = Arc::new(Hook::silent("second"));
    let hooks = vec![as_hook(Arc::clone(&first)), as_hook(Arc::clone(&second))];

    let modified =
        pre_call_deployment_hook(&hooks, &request(r#"{"model":"gpt-4o"}"#), "completion")
            .await
            .expect("the hooks accept the call");

    assert_eq!(modified.get("marker"), Some(&Json::Bool(true)));
    assert_eq!(*first.pre_calls.lock().expect("the counter is free"), 1);
    assert_eq!(*second.pre_calls.lock().expect("the counter is free"), 1);
}

#[tokio::test]
async fn deployment_hook_pre_call_error_goes_to_the_caller() {
    let mut spec = Hook::silent("broken");
    spec.fault = Fault::Pre;
    let hooks = vec![arc(spec)];

    let error = pre_call_deployment_hook(&hooks, &request("{}"), "completion")
        .await
        .expect_err("the hook error surfaces");

    assert_eq!(
        error.to_string(),
        "the broken deployment hook failed: pre-call hook failed"
    );
}

#[tokio::test]
async fn deployment_hook_success_ignores_a_logger_error() {
    let mut spec = Hook::silent("broken");
    spec.fault = Fault::Success;
    let broken = Arc::new(spec);
    let healthy = Arc::new(Hook::silent("healthy"));
    let hooks = vec![as_hook(Arc::clone(&broken)), as_hook(Arc::clone(&healthy))];
    let response = Json::Text(String::from("response"));

    let outcome = success_deployment_hook(&hooks, &request("{}"), &response, "completion")
        .await
        .expect("a broken logger does not fail the request");

    assert_eq!(outcome, response);
    assert_eq!(
        *healthy.success_calls.lock().expect("the counter is free"),
        1
    );
}

#[tokio::test]
async fn deployment_hook_success_surfaces_a_guardrail_error() {
    let mut spec = Hook::silent("guardrail");
    spec.guardrail = Some(Guardrail {
        name: Some(String::from("pii")),
        default_on: true,
    });
    spec.fault = Fault::Success;
    let hooks = vec![arc(spec)];

    let error = success_deployment_hook(
        &hooks,
        &request("{}"),
        &Json::Text(String::from("response")),
        "completion",
    )
    .await
    .expect_err("a guardrail error surfaces");

    assert_eq!(
        error.to_string(),
        "the guardrail deployment hook failed: success hook failed"
    );
}

#[tokio::test]
async fn deployment_hook_success_takes_a_replacement() {
    let mut spec = Hook::silent("hook");
    spec.success_replacement = Some(Json::Text(String::from("replacement")));
    let hooks = vec![arc(spec)];

    let outcome = success_deployment_hook(
        &hooks,
        &request("{}"),
        &Json::Text(String::from("response")),
        "completion",
    )
    .await
    .expect("the hook accepts the response");

    assert_eq!(outcome, Json::Text(String::from("replacement")));
}

#[tokio::test]
async fn deployment_hook_failure_never_surfaces_an_error() {
    let mut spec = Hook::silent("broken");
    spec.fault = Fault::FailureHook;
    let hooks = vec![arc(spec)];

    failure_deployment_hook(&hooks, &request("{}"), &failure(), "completion").await;
}

#[tokio::test]
async fn deployment_hook_failure_hides_the_attempted_targets() {
    let hook = Arc::new(Hook::silent("hook"));
    let hooks = vec![as_hook(Arc::clone(&hook))];

    failure_deployment_hook(
        &hooks,
        &request(r#"{"model":"gpt-4o","attempted_targets":["a"],"fallback_depth":2}"#),
        &failure(),
        "completion",
    )
    .await;

    assert_eq!(
        hook.saw_targets
            .lock()
            .expect("the target log is free")
            .as_slice(),
        [false]
    );
    assert_eq!(
        hook.depths
            .lock()
            .expect("the depth log is free")
            .as_slice(),
        [Some(2)]
    );
    assert_eq!(
        hook.saw_model
            .lock()
            .expect("the model log is free")
            .as_slice(),
        [true]
    );
}

#[tokio::test]
async fn deployment_hook_failure_ignores_a_text_fallback_depth() {
    let hook = Arc::new(Hook::silent("hook"));
    let hooks = vec![as_hook(Arc::clone(&hook))];

    failure_deployment_hook(
        &hooks,
        &request(r#"{"fallback_depth":"2"}"#),
        &failure(),
        "completion",
    )
    .await;

    assert_eq!(
        hook.depths
            .lock()
            .expect("the depth log is free")
            .as_slice(),
        [None]
    );
}

#[tokio::test]
async fn deployment_hook_failure_runs_every_hook() {
    let mut spec = Hook::silent("first");
    spec.fault = Fault::FailureHook;
    let first = Arc::new(spec);
    let second = Arc::new(Hook::silent("second"));
    let hooks = vec![as_hook(Arc::clone(&first)), as_hook(Arc::clone(&second))];

    failure_deployment_hook(&hooks, &request("{}"), &failure(), "completion").await;

    assert_eq!(
        second
            .saw_targets
            .lock()
            .expect("the target log is free")
            .len(),
        1
    );
}

#[tokio::test]
async fn deployment_hook_converted_stream_takes_a_replacement() {
    let mut spec = Hook::silent("hook");
    spec.success_replacement = Some(Json::Text(String::from("hooked")));
    let hooks = vec![arc(spec)];
    let mut outcome = CallOutcome::ChatStream(ChatStream {
        model_response: Json::Text(String::from("original")),
        json_mode: true,
        converted_from_mock: true,
    });

    run_success_hook_on_stream(&hooks, &mut outcome, &request("{}"), "completion")
        .await
        .expect("the hook accepts the stream");

    let CallOutcome::ChatStream(stream) = outcome else {
        panic!("the outcome stays a chat stream");
    };
    assert_eq!(stream.model_response, Json::Text(String::from("hooked")));
    assert!(stream.json_mode);
}

#[tokio::test]
async fn deployment_hook_converted_stream_skips_a_plain_response() {
    let mut spec = Hook::silent("hook");
    spec.fault = Fault::Success;
    let hooks = vec![arc(spec)];
    let mut outcome = CallOutcome::Response(Json::Text(String::from("original")));

    run_success_hook_on_stream(&hooks, &mut outcome, &request("{}"), "completion")
        .await
        .expect("a finished response never reaches the hook");
}

#[tokio::test]
async fn deployment_hook_converted_stream_skips_a_non_mock_stream() {
    let mut spec = Hook::silent("hook");
    spec.fault = Fault::Success;
    let hooks = vec![arc(spec)];
    let mut outcome = CallOutcome::ChatStream(ChatStream {
        model_response: Json::Text(String::from("original")),
        json_mode: false,
        converted_from_mock: false,
    });

    run_success_hook_on_stream(&hooks, &mut outcome, &request("{}"), "completion")
        .await
        .expect("only a converted mock stream reaches the hook");
}

#[tokio::test]
async fn deployment_hook_converted_stream_skips_an_unknown_call_type() {
    let mut spec = Hook::silent("hook");
    spec.fault = Fault::Success;
    let hooks = vec![arc(spec)];
    let mut outcome = CallOutcome::ChatStream(ChatStream {
        model_response: Json::Text(String::from("original")),
        json_mode: false,
        converted_from_mock: true,
    });

    run_success_hook_on_stream(&hooks, &mut outcome, &request("{}"), "some_new_call")
        .await
        .expect("an unknown call type skips the hook");
}

#[tokio::test]
async fn deployment_hook_pre_call_default_leaves_the_request() {
    let marker = Arc::new(Marker {
        label: String::from("plain"),
        guardrail: None,
    });
    let hooks = vec![as_hook(Arc::clone(&marker))];

    let modified =
        pre_call_deployment_hook(&hooks, &request(r#"{"model":"gpt-4o"}"#), "completion")
            .await
            .expect("the default hook accepts the call");

    assert_eq!(
        modified.get("model"),
        Some(&Json::Text(String::from("gpt-4o")))
    );
}

#[tokio::test]
async fn deployment_hook_success_default_leaves_the_response() {
    let marker = Arc::new(Marker {
        label: String::from("plain"),
        guardrail: None,
    });
    let hooks = vec![as_hook(Arc::clone(&marker))];
    let response = Json::Text(String::from("response"));

    let outcome = success_deployment_hook(&hooks, &request("{}"), &response, "completion")
        .await
        .expect("the default hook accepts the response");

    assert_eq!(outcome, response);
}
