use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::inputs::GuardrailApiInputs;
use betterlite_guardrails::promptguard::PromptGuardGuardrail;
use betterlite_guardrails::promptguard::init::PromptGuardArgs;
use betterlite_http::proxy::base_builder;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::case::UpstreamAnswer;
use crate::helpers::{Helper, HelperError, HelperResult};
use crate::mock::MockProvider;
use crate::routing_mock_scripts::MockScript;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) api_key: String,
    #[serde(default)]
    pub(crate) block_on_error: Option<bool>,
    pub(crate) inputs: GuardrailApiInputs,
    pub(crate) input_type: GuardrailInputType,
    pub(crate) answer: UpstreamAnswer,
}

#[derive(Debug, Serialize)]
struct Output {
    request: RequestView,
    #[serde(skip_serializing_if = "Option::is_none")]
    inputs: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    raised: Option<RaisedView>,
}

#[derive(Debug, Serialize)]
struct RequestView {
    method: String,
    path: String,
    body: Value,
    content_type: Option<String>,
    api_key_present: bool,
}

#[derive(Debug, Serialize)]
struct RaisedView {
    status: u16,
    blocked_content: bool,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let output = execute(input)?;
        serde_json::to_value(output).map_err(|error| fail("SerializationError", error.to_string()))
    }
}

fn execute(input: &Input) -> Result<Output, Box<HelperError>> {
    let (mock, base) = start_mock(&input.answer)?;
    let guardrail = build_guardrail(input, base)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| fail("OSError", error.to_string()))?;
    let outcome =
        runtime.block_on(guardrail.apply_guardrail(input.inputs.clone(), input.input_type));
    let calls = mock
        .finish()
        .map_err(|error| fail("MockError", error.to_string()))?;
    let call = calls
        .into_iter()
        .next()
        .ok_or_else(|| fail("MockError", "the guardrail made no request".to_owned()))?;
    let mut output = Output {
        request: RequestView {
            api_key_present: call.request.headers.contains_key("x-api-key"),
            content_type: call.request.headers.get("content-type").cloned(),
            method: call.request.method,
            path: call.request.path,
            body: call.request.body,
        },
        inputs: None,
        raised: None,
    };
    match outcome {
        Ok(updated) => {
            output.inputs = Some(
                serde_json::to_value(updated)
                    .map_err(|error| fail("SerializationError", error.to_string()))?,
            );
        }
        Err(error) => {
            output.raised = Some(RaisedView {
                status: error.status(),
                blocked_content: error.blocked_content(),
            });
        }
    }
    Ok(output)
}

fn build_guardrail(
    input: &Input,
    api_base: String,
) -> Result<PromptGuardGuardrail, Box<HelperError>> {
    let args = PromptGuardArgs {
        api_base: Some(api_base),
        api_key: Some(SecretString::from(input.api_key.clone())),
        block_on_error: input.block_on_error,
        timeout: None,
        event_hook: LitellmParamsMode::One("pre_call".to_owned()),
        default_on: false,
        name: "promptguard-test".to_owned(),
    };
    let client = base_builder()
        .build()
        .map_err(|error| fail("RequestError", error.to_string()))?;
    let env = BTreeMap::<String, String>::new();
    PromptGuardGuardrail::new(args, &env, client)
        .map_err(|error| fail("PromptGuardError", error.to_string()))
}

fn start_mock(answer: &UpstreamAnswer) -> Result<(MockProvider, String), Box<HelperError>> {
    let script = MockScript::try_from(std::slice::from_ref(answer))
        .map_err(|error| fail("MockError", error.to_string()))?;
    let mock = MockProvider::start(script).map_err(|error| fail("MockError", error.to_string()))?;
    let base = mock.base_url.clone();
    Ok((mock, base))
}

fn fail(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
