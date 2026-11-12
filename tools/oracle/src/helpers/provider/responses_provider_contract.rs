use betterlite_models::decisions::Json;
use betterlite_provider::api::responses_provider_contract::config::ResponsesProviderConfig as _;
use betterlite_provider::api::responses_provider_contract::error::{
    BaseLlmException, ResponsesContractError,
};
use betterlite_provider::api::responses_provider_contract::records::{
    CreateRequest, Headers, InputItemsQuery, Params, SignRequest, api_base_or_required,
    normalize_responses_request,
};
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
            Input::BaseErrorFields(fields) => {
                let fixture = Fixture::default();
                Ok(exception_fields(&exception_of(raise_class(
                    &fixture, fields,
                ))?))
            }
        }
    }
}

fn capabilities(fields: &BaseCapabilitiesInput) -> crate::helpers::HelperResult {
    let fixture = Fixture::default();
    let query = serde_json::to_value(InputItemsQuery::default())?;
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
    let (headers, signed_body) = fixture.sign_request(SignRequest {
        headers: fields.headers.clone(),
        optional_params: fields.optional_params.clone(),
        request_data: fields.request_data.clone(),
        api_base: fields.api_base.clone(),
        api_key: fields.api_key.clone().map(SecretString::from),
        model: Some(fields.model.clone()),
        stream: fields.stream,
        fake_stream: fields.fake_stream,
    });
    let extra_body = fixture.transform_extra_body(
        &fields.extra_body,
        &fields.request,
        &fields.model,
        &Params::new(),
    );
    let environment = fixture.validate_environment(&fields.headers, &fields.model, None)?;
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
    let mut steps = Vec::with_capacity(fields.steps.len());
    for step in &fields.steps {
        steps.push(serde_json::to_value(normalize_responses_request(step))?);
    }
    Ok(Value::Array(steps))
}

fn block_on_create(
    fixture: &Fixture,
    fields: &ForwardingInput,
) -> Result<Params, Box<crate::helpers::HelperError>> {
    let request = CreateRequest {
        model: fields.model.clone(),
        input: fields.input.clone(),
        optional_params: fields.optional_params.clone(),
        litellm_params: fields.litellm_params.clone(),
        headers: fields.headers.clone(),
    };
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(fixture.transform_create_request_async(&request))
        .map_err(Into::into)
}

fn forwarded_calls(fixture: &Fixture) -> Result<Vec<Value>, Box<crate::helpers::HelperError>> {
    let mut calls = Vec::with_capacity(fixture.forwarded().len());
    for call in &fixture.forwarded() {
        calls.push(serde_json::to_value(call)?);
    }
    Ok(calls)
}

fn forwarding(fields: &ForwardingInput) -> crate::helpers::HelperResult {
    let fixture = Fixture::default();
    let result = block_on_create(&fixture, fields)?;
    let calls = forwarded_calls(&fixture)?;
    Ok(json!({"calls": calls, "result": result}))
}

fn raise_class(fixture: &Fixture, fields: &ErrorInput) -> FixtureError {
    match fixture.error_class(&fields.message, fields.status_code, fields.headers.clone()) {
        Err(error) => error,
        Ok(never) => match never {},
    }
}

fn exception_of(error: FixtureError) -> Result<BaseLlmException, Box<crate::helpers::HelperError>> {
    match error {
        FixtureError::Contract(ResponsesContractError::BaseLlm(exception)) => Ok(exception),
        other @ (FixtureError::Contract(_) | FixtureError::Unused) => {
            Err(Box::<crate::helpers::HelperError>::from(other))
        }
    }
}

fn exception_fields(exception: &BaseLlmException) -> Value {
    json!({
        "raised": true,
        "status_code": exception.status_code,
        "message": exception.message,
        "headers": exception.headers,
    })
}
