use super::support::{Configs, Harness, Nothing, Source, environment, round_trip};
use crate::api::responses_create::bridge::Created;
use crate::api::responses_create::call::ResponsesCreateCall;
use crate::api::responses_create::deps::ResponsesCreateDeps;
use crate::api::responses_create::error::{CreateError, CreateFailure};
use crate::api::responses_create::execute::create;
use crate::api::responses_create_http::service::{CreateRequest, LogContext};
use crate::api::responses_provider_selection::ParameterMap;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::response_input::{ResponsesInput, ResponsesTool};
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use std::collections::BTreeMap;

pub(crate) fn call(model: &str) -> ResponsesCreateCall {
    ResponsesCreateCall {
        model: model.to_owned(),
        input: ResponsesInput::Text(String::from("hello")),
        include: None,
        instructions: None,
        max_output_tokens: None,
        prompt: None,
        metadata: None,
        parallel_tool_calls: None,
        previous_response_id: None,
        reasoning: None,
        store: None,
        background: None,
        stream: None,
        temperature: None,
        text: None,
        text_format: None,
        tool_choice: None,
        tools: None,
        top_p: None,
        truncation: None,
        user: None,
        service_tier: None,
        safety_identifier: None,
        extra_headers: None,
        extra_query: None,
        extra_body: None,
        timeout: None,
        allowed_openai_params: None,
        custom_llm_provider: None,
        api_key: None,
        api_base: Some(String::from("http://mock.test/v1")),
        ssl_verify: None,
        shared_session: None,
        kwargs: ParameterMap::new(),
    }
}

pub(crate) fn kwargs(text: &str) -> ParameterMap {
    serde_json::from_str(text).expect("the kwargs are JSON")
}

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the JSON parses")
}

pub(crate) fn tools(text: &str) -> Vec<ResponsesTool> {
    serde_json::from_str(text).expect("the tools are JSON")
}

pub(crate) fn item_input(text: &str) -> ResponsesInput {
    serde_json::from_str(text).expect("the item input is JSON")
}

pub(crate) fn values(text: &str) -> Vec<JsonValue> {
    serde_json::from_str(text).expect("the values are JSON")
}

pub(crate) async fn run_create(
    deps: &ResponsesCreateDeps<'_, Source, Configs, Nothing, Nothing, ()>,
    call: &ResponsesCreateCall,
    harness: &mut Harness,
) -> Result<Created<()>, CreateError> {
    let env = environment(&harness.redactor, &harness.sink, harness.now);
    let mut log = LogContext {
        state: &mut harness.state,
        env: &env,
    };
    create(deps, call, (), &mut log).await
}

pub(crate) fn env_vars(values: &[(&str, &str)]) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn body_json(request: &CreateRequest) -> PromptJson {
    let RequestBody::Json(bytes) = &request.body else {
        panic!("the create request must carry a JSON body");
    };
    serde_json::from_slice(bytes).expect("the create request body is JSON")
}

pub(crate) fn object(value: &PromptJson) -> Object {
    let PromptJson::Object(fields) = value else {
        panic!("the value must be an object");
    };
    fields.clone()
}

pub(crate) fn text(fields: &Object, name: &str) -> String {
    let Some(PromptJson::Text(found)) = fields.get(name) else {
        panic!("the field must hold text");
    };
    found.clone()
}

pub(crate) fn field<'value>(fields: &'value Object, name: &str) -> &'value PromptJson {
    fields.get(name).unwrap_or_else(|| {
        panic!("the field must exist");
    })
}

pub(crate) fn list(value: &PromptJson) -> Vec<PromptJson> {
    let PromptJson::List(items) = value else {
        panic!("the value must be a list");
    };
    items.clone()
}

pub(crate) fn number_is(fields: &Object, name: &str, expected: f64) {
    let Some(PromptJson::Number(found)) = fields.get(name) else {
        panic!("the field must hold a number");
    };
    let parsed = found.as_f64().expect("the number is finite");
    assert!((parsed - expected).abs() < 1e-9_f64, "{name}: {parsed}");
}

pub(crate) fn value_number(map: &ParameterMap, name: &str) -> f64 {
    let Some(JsonValue::Number(found)) = map.get(name) else {
        panic!("the field must hold a number");
    };
    found.as_f64().expect("the number is finite")
}

pub(crate) fn response_json(created: &Created<()>) -> PromptJson {
    let Created::Responses(stamped) = created else {
        panic!("the create answer must carry a typed response");
    };
    round_trip(&stamped.response).expect("the response serializes")
}

pub(crate) fn header_values(request: &CreateRequest, name: &str) -> Vec<String> {
    request
        .headers
        .iter()
        .filter(|(candidate, _value)| candidate.as_str().eq_ignore_ascii_case(name))
        .map(|(_name, value)| {
            value
                .to_str()
                .expect("the header value is visible text")
                .to_owned()
        })
        .collect()
}

pub(crate) fn failure(error: &CreateError) -> CreateFailure {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
    error.sdk_failure("openai", "gpt-4o", &redactor)
}
