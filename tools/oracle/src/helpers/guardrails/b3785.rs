use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_guardrails::javelin::AppliedGuardrailsHeader;
use betterlite_guardrails::pangea::{
    GuardrailResponse, PangeaGuardrail, PangeaGuardrailConfigModel, PangeaGuardrailError,
    PangeaOptionalParams, RequestData,
};
use betterlite_http::proxy::base_builder;
use serde_json::{Value, json};

use crate::case::UpstreamAnswer;
use crate::helpers::{HelperError, HelperResult};
use crate::mock::MockProvider;
use crate::routing_mock_scripts::{MockScript, Outcome, ScriptSpec, ScriptedReply, Wait};

mod request;

use request::{Input, PostCall, PreCall, ResponseKind};

const COMPLETION_CALL_TYPES: [&str; 2] = ["text_completion", "atext_completion"];
const HANGING_RESPONSE_MILLIS: u64 = 5_000;
const HANGING_SCRIPT_DEADLINE_MILLIS: u64 = 30_000;
const HANGING_CLIENT_TIMEOUT: Duration = Duration::from_millis(100);

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::PreCall(call) => run_pre_call(call, false),
            Input::BackendTimeout(call) => run_pre_call(call, true),
            Input::PostCall(call) => run_post_call(call),
        }
    }
}

struct Recorder;

impl AppliedGuardrailsHeader for Recorder {
    fn record_applied_guardrail(&mut self, _guardrail_name: &str) {}
}

fn fail(class: &str, message: String, status: Option<u16>) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn runtime() -> Result<tokio::runtime::Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| fail("OSError", error.to_string(), None))
}

struct Endpoint<'call> {
    api_key: &'call str,
    guardrail_name: &'call str,
    input_recipe: Option<&'call String>,
    output_recipe: Option<&'call String>,
    base: &'call str,
    hanging: bool,
}

impl Endpoint<'_> {
    fn guardrail(&self) -> Result<PangeaGuardrail, Box<HelperError>> {
        let config = PangeaGuardrailConfigModel {
            api_key: Some(self.api_key.to_owned()),
            api_base: Some(self.base.to_owned()),
            optional_params: Some(PangeaOptionalParams {
                pangea_input_recipe: self.input_recipe.cloned(),
                pangea_output_recipe: self.output_recipe.cloned(),
            }),
        };
        let builder = base_builder();
        let configured = if self.hanging {
            builder.timeout(HANGING_CLIENT_TIMEOUT)
        } else {
            builder
        };
        let client = configured
            .build()
            .map_err(|error| fail("RequestError", error.to_string(), None))?;
        PangeaGuardrail::new(
            Some(self.guardrail_name.to_owned()),
            config,
            &BTreeMap::new(),
            client,
        )
        .map_err(|error| fail("ValueError", error.to_string(), None))
    }
}

fn start_mock(
    answer: Option<&UpstreamAnswer>,
    hanging: bool,
) -> Result<(MockProvider, String), Box<HelperError>> {
    let script = if hanging {
        let reply = ScriptedReply {
            wait: Wait::Delay {
                millis: HANGING_RESPONSE_MILLIS,
            },
            then: Outcome::Answer(UpstreamAnswer {
                status: 200,
                headers: BTreeMap::new(),
                body: Some(json!({})),
                events: None,
            }),
        };
        MockScript::try_from(ScriptSpec {
            deadline_ms: HANGING_SCRIPT_DEADLINE_MILLIS,
            replies: vec![reply],
        })
    } else {
        let reply =
            answer.ok_or_else(|| fail("MockError", "the case has no answer".to_owned(), None))?;
        MockScript::try_from(std::slice::from_ref(reply))
    }
    .map_err(|error| fail("MockError", error.to_string(), None))?;
    let mock =
        MockProvider::start(script).map_err(|error| fail("MockError", error.to_string(), None))?;
    let base = mock.base_url.clone();
    Ok((mock, base))
}

type Call = (Option<String>, Option<Value>);

fn take_call(mock: MockProvider) -> Result<Call, Box<HelperError>> {
    let calls = mock
        .finish()
        .map_err(|error| fail("MockError", error.to_string(), None))?;
    Ok(calls.into_iter().next().map_or((None, None), |call| {
        (Some(call.request.path), Some(call.request.body))
    }))
}

fn failed(error: &PangeaGuardrailError) -> Box<HelperError> {
    fail(
        "HTTPException",
        error.to_string(),
        Some(error.status_code().as_u16()),
    )
}

fn blocked_or_failed(
    error: &PangeaGuardrailError,
    guardrail_name: &str,
    path: Option<&String>,
    request: Option<&Value>,
) -> HelperResult {
    if matches!(error, PangeaGuardrailError::Blocked) {
        let detail = error.detail(guardrail_name);
        return Ok(json!({
            "path": path,
            "request": request,
            "blocked": true,
            "status": error.status_code().as_u16(),
            "detail": detail,
        }));
    }
    Err(failed(error))
}

fn pre_call_output(mut output: Value, call_type: &str, data: &RequestData) -> Value {
    if let Value::Object(fields) = &mut output {
        if COMPLETION_CALL_TYPES.contains(&call_type) {
            let _inserted = fields.insert("prompt".to_owned(), json!(data.prompt));
        } else {
            let _inserted = fields.insert("messages".to_owned(), json!(data.messages));
        }
    }
    output
}

fn post_call_output(mut output: Value, response: &GuardrailResponse) -> Value {
    let choices = match response {
        GuardrailResponse::Model { choices } | GuardrailResponse::TextCompletion { choices } => {
            choices
        }
    };
    if let Value::Object(fields) = &mut output {
        let _inserted = fields.insert("choices".to_owned(), json!(choices));
    }
    output
}

fn run_pre_call(call: &PreCall, hanging: bool) -> HelperResult {
    let (mock, base) = start_mock(call.answer.as_ref(), hanging)?;
    let endpoint = Endpoint {
        api_key: &call.api_key,
        guardrail_name: &call.guardrail_name,
        input_recipe: call.input_recipe.as_ref(),
        output_recipe: call.output_recipe.as_ref(),
        base: &base,
        hanging,
    };
    let guardrail = endpoint.guardrail()?;
    let mut data = RequestData {
        messages: call.messages.clone(),
        prompt: call.prompt.clone(),
        tools: call.tools.clone(),
    };
    let mut recorder = Recorder;
    let hook = guardrail.async_pre_call_hook(&mut data, &call.call_type, &mut recorder);
    let result = runtime()?.block_on(hook);
    let (path, request) = take_call(mock)?;
    match result {
        Ok(true) => Ok(pre_call_output(
            json!({"path": path, "request": request}),
            &call.call_type,
            &data,
        )),
        Ok(false) => Ok(json!({"path": path, "request": request})),
        Err(error) => blocked_or_failed(
            &error,
            &call.guardrail_name,
            path.as_ref(),
            request.as_ref(),
        ),
    }
}

fn run_post_call(call: &PostCall) -> HelperResult {
    let (mock, base) = start_mock(call.answer.as_ref(), false)?;
    let endpoint = Endpoint {
        api_key: &call.api_key,
        guardrail_name: &call.guardrail_name,
        input_recipe: call.input_recipe.as_ref(),
        output_recipe: call.output_recipe.as_ref(),
        base: &base,
        hanging: false,
    };
    let guardrail = endpoint.guardrail()?;
    let mut data = RequestData {
        messages: call.messages.clone(),
        prompt: call.prompt.clone(),
        tools: call.tools.clone(),
    };
    let mut response = match call.kind {
        ResponseKind::Model => GuardrailResponse::Model {
            choices: call.choices.clone(),
        },
        ResponseKind::TextCompletion => GuardrailResponse::TextCompletion {
            choices: call.choices.clone(),
        },
    };
    let mut recorder = Recorder;
    let hook = guardrail.async_post_call_success_hook(&mut data, &mut response, &mut recorder);
    let result = runtime()?.block_on(hook);
    let (path, request) = take_call(mock)?;
    match result {
        Ok(true) => Ok(post_call_output(
            json!({"path": path, "request": request}),
            &response,
        )),
        Ok(false) => Ok(json!({"path": path, "request": request})),
        Err(error) => blocked_or_failed(
            &error,
            &call.guardrail_name,
            path.as_ref(),
            request.as_ref(),
        ),
    }
}
