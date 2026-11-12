use betterlite_models::decisions::Json;
use betterlite_provider::api::responses_provider_contract::config::{
    CreateRequest, Headers, InputItemsQuery, Params, ResponsesProviderConfig as _, SignRequest,
    api_base_or_required, normalize_responses_request,
};
use betterlite_provider::api::responses_provider_contract::error::ResponsesContractError;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use fixtures::{Fixture, FixtureError};

mod fixtures;
mod tests;

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum Input {
    BaseCapabilities(BaseCapabilitiesInput),
    SigningPassthrough(SigningInput),
    WebsocketUrl(WebsocketInput),
    RequestNormalization(NormalizationInput),
    AsyncRequestForwarding(ForwardingInput),
    BaseErrorFields(ErrorInput),
}

#[derive(Debug, Deserialize)]
pub(crate) struct BaseCapabilitiesInput {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    stream: Option<bool>,
    #[serde(default)]
    custom_llm_provider: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SigningInput {
    model: String,
    headers: Headers,
    optional_params: Params,
    request_data: Params,
    api_base: String,
    api_key: Option<String>,
    stream: Option<bool>,
    fake_stream: Option<bool>,
    extra_body: Params,
    request: Params,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WebsocketInput {
    api_bases: Vec<Option<String>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct NormalizationInput {
    steps: Vec<Params>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ForwardingInput {
    model: String,
    input: Json,
    optional_params: Params,
    litellm_params: Params,
    headers: Headers,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ErrorInput {
    message: String,
    status_code: u16,
    headers: Headers,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::BaseCapabilities(fields) => capabilities(fields),
            Input::SigningPassthrough(fields) => signing(fields),
            Input::WebsocketUrl(fields) => Ok(websocket(fields)),
            Input::RequestNormalization(fields) => normalization(fields),
            Input::AsyncRequestForwarding(fields) => forwarding(fields),
            Input::BaseErrorFields(fields) => error_fields(fields),
        }
    }
}

fn capabilities(fields: &BaseCapabilitiesInput) -> crate::helpers::HelperResult {
    let fixture = Fixture::default();
    let query = serde_json::to_value(InputItemsQuery::default())
        .map_err(|error| helper_error(error.to_string()))?;
    Ok(json!({
        "provider": fixture.provider().name(),
        "native_file_search": fixture.supports_native_file_search(),
        "encrypted_agent_messages": fixture.supports_encrypted_messages(),
        "native_websocket": fixture.supports_native_websocket(),
        "model_in_websocket_url": fixture.model_in_websocket_url(),
        "should_fake_stream": fixture.should_fake_stream(
            fields.model.as_deref(),
            fields.stream,
            fields.custom_llm_provider.as_deref(),
        ),
        "input_items": query,
    }))
}

fn signing(fields: &SigningInput) -> crate::helpers::HelperResult {
    let fixture = Fixture::default();
    let api_key = fields.api_key.as_deref().map(SecretString::from);
    let (headers, signed_body) = fixture.sign_request(SignRequest {
        headers: &fields.headers,
        optional_params: &fields.optional_params,
        request_data: &fields.request_data,
        api_base: &fields.api_base,
        api_key: api_key.as_ref(),
        model: Some(fields.model.as_str()),
        stream: fields.stream,
        fake_stream: fields.fake_stream,
    });
    let extra_body = fixture.transform_extra_body(
        &fields.extra_body,
        &fields.request,
        &fields.model,
        &Params::new(),
    );
    let environment = fixture
        .validate_environment(&fields.headers, &fields.model, None)
        .map_err(|error| helper_error(error.to_string()))?;
    Ok(json!({
        "headers": headers,
        "signed_body": signed_body,
        "extra_body": extra_body,
        "environment": environment,
    }))
}

fn websocket(fields: &WebsocketInput) -> Value {
    let fixture = Fixture::default();
    let params = Params::new();
    let mut steps = Vec::new();
    for api_base in &fields.api_bases {
        let complete = match api_base_or_required(api_base.as_deref()) {
            Ok(url) => Value::String(url.to_owned()),
            Err(error) => json!({"error": {"class": "ValueError", "message": error.to_string()}}),
        };
        let websocket = match fixture.websocket_url(api_base.as_deref(), &params) {
            Ok(url) => Value::String(url),
            Err(error) => json!({"error": {"class": "ValueError", "message": error.to_string()}}),
        };
        steps.push(json!({
            "complete_url": complete,
            "websocket_url": websocket,
            "websocket_calls": fixture.complete_url_calls(),
        }));
    }
    Value::Array(steps)
}

fn normalization(fields: &NormalizationInput) -> crate::helpers::HelperResult {
    let results: Vec<Value> = fields
        .steps
        .iter()
        .map(|step| {
            serde_json::to_value(normalize_responses_request(step))
                .map_err(|error| helper_error(error.to_string()))
        })
        .collect::<Result<_, _>>()?;
    Ok(Value::Array(results))
}

fn forwarding(fields: &ForwardingInput) -> crate::helpers::HelperResult {
    let fixture = Fixture::default();
    let request = CreateRequest {
        model: &fields.model,
        input: &fields.input,
        optional_params: &fields.optional_params,
        litellm_params: &fields.litellm_params,
        headers: &fields.headers,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| helper_error(error.to_string()))?;
    let result = runtime
        .block_on(fixture.transform_create_request_async(&request))
        .map_err(|error| helper_error(error.to_string()))?;
    let calls: Vec<Value> = fixture
        .forwarded()
        .iter()
        .map(|call| serde_json::to_value(call).map_err(|error| helper_error(error.to_string())))
        .collect::<Result<_, _>>()?;
    Ok(json!({"calls": calls, "result": result}))
}

fn error_fields(fields: &ErrorInput) -> crate::helpers::HelperResult {
    let fixture = Fixture::default();
    let error =
        match fixture.error_class(&fields.message, fields.status_code, fields.headers.clone()) {
            Err(error) => error,
            Ok(never) => match never {},
        };
    let exception = match error {
        FixtureError::Contract(ResponsesContractError::BaseLlm(exception)) => exception,
        FixtureError::Contract(other) => return Err(helper_error(other.to_string())),
        unused @ FixtureError::Unused => return Err(helper_error(unused.to_string())),
    };
    Ok(json!({
        "raised": true,
        "status_code": exception.status_code,
        "message": exception.message,
        "headers": exception.headers,
    }))
}

fn helper_error(message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: String::from("ValueError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
