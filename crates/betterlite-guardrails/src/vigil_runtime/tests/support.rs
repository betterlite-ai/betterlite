#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::vigil_runtime::Settings as EnvironmentDefaults;
use reqwest::header::HeaderMap;
use secrecy::ExposeSecret as _;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::contract::GuardrailContract;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::third_party_init_b::vigil_guard::VigilGuardArgs;
use crate::vigil_guard::analysis::{AnalyzeMode, AnalyzePayload, Source};
use crate::vigil_guard::handler_protocol::{AsyncPostHandler, HttpResponse, RequestTimeout};
use crate::vigil_guard::metadata::MetadataValue;
use crate::vigil_guard::transient::TransportError;
use crate::vigil_runtime::error::VigilRuntimeError;
use crate::vigil_runtime::{SUPPORTED_EVENT_HOOKS, VIGIL_SWITCHES, VigilGuard, VigilGuardParams};

use super::cases::{BuildCase, BuildExpectation, BuiltSettings, Expected, Scenario};

#[derive(Debug, Clone)]
pub(crate) struct Call {
    pub(crate) payload: AnalyzePayload,
}

pub(crate) struct FakeHandler {
    calls: Mutex<Vec<Call>>,
    responses: Mutex<VecDeque<Result<HttpResponse, TransportError>>>,
}

impl FakeHandler {
    pub(crate) fn new(responses: Vec<Result<HttpResponse, TransportError>>) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            responses: Mutex::new(VecDeque::from(responses)),
        }
    }

    pub(crate) fn calls(&self) -> Vec<Call> {
        lock(&self.calls).clone()
    }
}

impl AsyncPostHandler for FakeHandler {
    fn post<'request>(
        &'request self,
        _url: &str,
        _headers: &HeaderMap,
        json: &AnalyzePayload,
        _timeout: RequestTimeout,
    ) -> Pin<Box<dyn Future<Output = Result<HttpResponse, TransportError>> + Send + 'request>> {
        lock(&self.calls).push(Call {
            payload: json.clone(),
        });
        let response = lock(&self.responses)
            .pop_front()
            .unwrap_or_else(|| Ok(json_body("{}")));
        Box::pin(async move { response })
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn json_body(body: &str) -> HttpResponse {
    HttpResponse {
        status: reqwest::StatusCode::OK,
        body: body.as_bytes().to_vec(),
    }
}

fn guardrail_name_of(args: &VigilGuardArgs) -> Option<String> {
    if args.name.is_empty() {
        return None;
    }
    Some(args.name.clone())
}

fn config_model() -> GuardrailValue {
    GuardrailValue::Object(BTreeMap::from([
        (
            "model".to_owned(),
            GuardrailValue::Text("VigilGuardGuardrailConfigModel".to_owned()),
        ),
        (
            "ui_friendly_name".to_owned(),
            GuardrailValue::Text("Vigil Guard".to_owned()),
        ),
    ]))
}

struct BaseValues {
    name: Option<String>,
    event_hook: Option<LitellmParamsMode>,
    default_on: bool,
    secret: Option<String>,
}

pub(crate) fn run_build(case: BuildCase) {
    let BuildCase {
        args,
        defaults,
        hooks,
        strict_modes,
        expected,
    } = case;
    let observed = BaseValues {
        name: guardrail_name_of(&args),
        event_hook: Some(args.event_hook.clone()),
        default_on: args.default_on,
        secret: args
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
    };
    let params = VigilGuardParams {
        args,
        defaults,
        supported_event_hooks: hooks,
        strict_modes,
        handler: Arc::new(FakeHandler::new(Vec::new())),
    };
    let outcome = VigilGuard::new(params);
    match expected {
        BuildExpectation::Built(built) => {
            check_built(
                &outcome.expect("the build case resolves"),
                &built,
                &observed,
            );
        }
        BuildExpectation::Error(fragment) => {
            check_error(outcome.err(), fragment, observed.secret.as_deref());
        }
    }
}

fn check_built(guardrail: &VigilGuard, built: &BuiltSettings, observed: &BaseValues) {
    assert_eq!(guardrail.settings.api_base, built.api_base);
    assert_eq!(guardrail.settings.api_key.expose_secret(), built.api_key);
    assert_eq!(guardrail.settings.timeout, built.timeout);
    assert_eq!(guardrail.unreachable_fallback, built.fallback);
    assert_eq!(
        guardrail.base.settings.supported_event_hooks.as_deref(),
        Some(built.hooks.as_slice())
    );
    assert_eq!(guardrail.base.settings.event_hook, observed.event_hook);
    assert_eq!(guardrail.base.settings.default_on, observed.default_on);
    assert_eq!(guardrail.base.settings.guardrail_name, observed.name);
    assert_eq!(guardrail.base.settings.switches, VIGIL_SWITCHES);
    assert!(guardrail.base.settings.sticky_session_routing);
    assert_eq!(
        guardrail.get_supported_event_hooks(),
        Some(SUPPORTED_EVENT_HOOKS.to_vec())
    );
    assert_eq!(guardrail.get_config_model(), Some(config_model()));
}

fn check_error(error: Option<VigilRuntimeError>, fragment: &str, secret: Option<&str>) {
    let Some(error) = error else {
        panic!("the build case should fail");
    };
    let message = error.to_string();
    assert!(
        message.contains(fragment),
        "the error should name {fragment}: {message}"
    );
    if let Some(secret) = secret {
        assert!(
            !message.contains(secret),
            "the error should not expose the key: {message}"
        );
    }
}

pub(crate) async fn run_scan(scenario: Scenario) {
    let Scenario {
        args,
        inputs,
        input_type,
        mut request_data,
        mut logging,
        responses,
        analyzed,
        metadata,
        expected,
    } = scenario;
    let expected_name = guardrail_name_of(&args);
    let original = inputs.clone();
    let handler = Arc::new(FakeHandler::new(responses));
    let params = VigilGuardParams {
        args,
        defaults: EnvironmentDefaults::default(),
        supported_event_hooks: None,
        strict_modes: true,
        handler: Arc::clone(&handler),
    };
    let guardrail = VigilGuard::new(params).expect("the scan scenario resolves");
    let result = guardrail
        .apply_guardrail(inputs, &mut request_data, input_type, logging.as_mut())
        .await;
    let calls = handler.calls();
    assert_calls(&calls, &analyzed, input_type, &metadata);
    assert_outcome(result, &original, expected, expected_name.as_deref());
}

fn assert_calls(
    calls: &[Call],
    analyzed: &[&str],
    input_type: GuardrailInputType,
    metadata: &[(&str, &str)],
) {
    let actual: Vec<&str> = calls
        .iter()
        .map(|call| call.payload.text.as_str())
        .collect();
    assert_eq!(actual, analyzed);
    let source = match input_type {
        GuardrailInputType::Request => Source::UserInput,
        GuardrailInputType::Response => Source::ModelOutput,
    };
    for call in calls {
        assert_eq!(call.payload.source, source);
        assert_eq!(call.payload.mode, AnalyzeMode::Full);
        for (name, value) in metadata {
            assert_eq!(
                call.payload.metadata.get(*name),
                Some(&MetadataValue::Text((*value).to_owned()))
            );
        }
    }
}

fn assert_outcome(
    result: Result<GuardrailApiInputs, GuardrailFailure>,
    original: &GuardrailApiInputs,
    expected: Expected,
    expected_name: Option<&str>,
) {
    match expected {
        Expected::Unchanged => assert_eq!(result, Ok(original.clone())),
        Expected::Rewritten { texts, tool_calls } => {
            assert_rewritten(result, &texts, tool_calls.as_deref(), original);
        }
        Expected::Raised {
            message,
            status_code,
            blocked_content,
        } => assert_raised(
            result,
            &message,
            status_code,
            blocked_content,
            expected_name,
        ),
        Expected::Other(fragment) => assert_other(result, &fragment),
    }
}

fn assert_rewritten(
    result: Result<GuardrailApiInputs, GuardrailFailure>,
    texts: &[String],
    tool_calls: Option<&[GuardrailValue]>,
    original: &GuardrailApiInputs,
) {
    let output = result.expect("the scan should rewrite");
    assert_eq!(output.texts.as_deref(), Some(texts));
    assert_eq!(output.tool_calls.as_deref(), tool_calls);
    assert_eq!(output.images, original.images);
    assert_eq!(output.tools, original.tools);
    assert_eq!(output.structured_messages, None);
    assert_eq!(output.model, None);
    assert_eq!(output.stream_holdback_chars, None);
}

fn assert_raised(
    result: Result<GuardrailApiInputs, GuardrailFailure>,
    message: &str,
    status_code: u16,
    blocked_content: bool,
    expected_name: Option<&str>,
) {
    match result {
        Err(GuardrailFailure::GuardrailRaised {
            guardrail_name,
            message: raised_message,
            wrap_default_message,
            status_code: raised_status,
            blocked_content: raised_blocked,
        }) => {
            assert_eq!(raised_message, message);
            assert_eq!(raised_status, status_code);
            assert_eq!(raised_blocked, blocked_content);
            assert!(!wrap_default_message);
            assert_eq!(guardrail_name.as_deref(), expected_name);
        }
        other => panic!("the scan should raise a guardrail failure: {other:?}"),
    }
}

fn assert_other(result: Result<GuardrailApiInputs, GuardrailFailure>, fragment: &str) {
    match result {
        Err(GuardrailFailure::Other(message)) => {
            assert!(
                message.contains(fragment),
                "the error should contain {fragment}: {message}"
            );
        }
        other => panic!("the scan should fail with a local error: {other:?}"),
    }
}
