use betterlite_guardrails::pangea::{GuardrailResponse, PangeaGuardrailError, RequestData};
use serde_json::{Value, json};

use crate::helpers::HelperResult;
use crate::mock::MockProvider;

mod request;
mod setup;

use request::{Input, PostCall, PreCall, ResponseKind};
use setup::{Common, fail, prepare, runtime};

const COMPLETION_CALL_TYPES: [&str; 2] = ["text_completion", "atext_completion"];

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

type Call = (Option<String>, Option<Value>);

fn take_call(mock: MockProvider) -> Result<Call, Box<crate::helpers::HelperError>> {
    let calls = mock
        .finish()
        .map_err(|error| fail("MockError", error.to_string(), None))?;
    Ok(calls.into_iter().next().map_or((None, None), |call| {
        (Some(call.request.path), Some(call.request.body))
    }))
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
    Err(fail(
        "HTTPException",
        error.to_string(),
        Some(error.status_code().as_u16()),
    ))
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
    let mut prepared = prepare(&Common {
        api_key: &call.api_key,
        guardrail_name: &call.guardrail_name,
        input_recipe: call.input_recipe.as_ref(),
        output_recipe: call.output_recipe.as_ref(),
        messages: &call.messages,
        prompt: &call.prompt,
        tools: &call.tools,
        answer: call.answer.as_ref(),
        hanging,
    })?;
    let hook = prepared.guardrail.async_pre_call_hook(
        &mut prepared.data,
        &call.call_type,
        &mut prepared.recorder,
    );
    let result = runtime()?.block_on(hook);
    let (path, request) = take_call(prepared.mock)?;
    match result {
        Ok(true) => Ok(pre_call_output(
            json!({"path": path, "request": request}),
            &call.call_type,
            &prepared.data,
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
    let mut response = match call.kind {
        ResponseKind::Model => GuardrailResponse::Model {
            choices: call.choices.clone(),
        },
        ResponseKind::TextCompletion => GuardrailResponse::TextCompletion {
            choices: call.choices.clone(),
        },
    };
    let mut prepared = prepare(&Common {
        api_key: &call.api_key,
        guardrail_name: &call.guardrail_name,
        input_recipe: call.input_recipe.as_ref(),
        output_recipe: call.output_recipe.as_ref(),
        messages: &call.messages,
        prompt: &call.prompt,
        tools: &call.tools,
        answer: call.answer.as_ref(),
        hanging: false,
    })?;
    let hook = prepared.guardrail.async_post_call_success_hook(
        &mut prepared.data,
        &mut response,
        &mut prepared.recorder,
    );
    let result = runtime()?.block_on(hook);
    let (path, request) = take_call(prepared.mock)?;
    if let Err(error) = &result {
        return blocked_or_failed(error, &call.guardrail_name, path.as_ref(), request.as_ref());
    }
    let transformed = matches!(result, Ok(true));
    let output = json!({"path": path, "request": request});
    if transformed {
        return Ok(post_call_output(output, &response));
    }
    Ok(output)
}
