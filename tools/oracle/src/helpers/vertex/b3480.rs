use std::collections::BTreeMap;

use betterlite_vertex::genai::genai_contract::config::{
    AuthTokenRequest, BaseGenerateContentConfig, EnvironmentRequest, GenerateContentRequest,
    RequestHeaders, ResponseHeaders, ResponseTransform,
};
use betterlite_vertex::genai::genai_contract::records::{
    GenerateContentConfigDict, GenerateContentRequestDict, GenerateContentResponse,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use thiserror::Error;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    #[serde(default)]
    constructs: Vec<ConstructCall>,
    #[serde(default)]
    top_level: Vec<TopLevelCall>,
    #[serde(default)]
    errors: Vec<ErrorCall>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConstructCall {
    record: Record,
    fields: Value,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TopLevelCall {
    provider: String,
    model: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ErrorCall {
    message: String,
    status_code: u16,
    headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Record {
    Response,
    Request,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut answers = Vec::new();
        for call in &input.constructs {
            answers.push(construct(call.record, &call.fields)?);
        }
        for call in &input.top_level {
            answers.push(json!({
                "provider": call.provider,
                "model": call.model,
                "fields": ProviderConfig.request_top_level_fields(),
            }));
        }
        for call in &input.errors {
            answers.push(error_record(call));
        }
        Ok(json!({ "calls": answers }))
    }
}

fn construct(record: Record, fields: &Value) -> Result<Value, Box<HelperError>> {
    match record {
        Record::Response => {
            let response: GenerateContentResponse = serde_json::from_value(fields.clone())?;
            Ok(json!({
                "value": serde_json::to_value(&response)?,
                "hidden_params": response.hidden_params,
            }))
        }
        Record::Request => {
            let request: GenerateContentRequestDict = serde_json::from_value(fields.clone())?;
            Ok(json!({ "value": serde_json::to_value(&request)? }))
        }
    }
}

fn error_record(call: &ErrorCall) -> Value {
    let sent: ResponseHeaders = call
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let error = ProviderConfig.get_error_class(&call.message, call.status_code, &sent);
    let mut rendered = Map::new();
    for (name, value) in error.headers.into_iter().flatten() {
        rendered.insert(name, Value::String(value));
    }
    json!({
        "status_code": error.status_code,
        "message": error.message,
        "headers": rendered,
    })
}

// the trait's provided methods are what the cases read, so a bare implementor stands in for a provider config
struct ProviderConfig;

#[derive(Debug, Error)]
enum FixtureError {
    #[error("the helper config does not transform requests")]
    Unused,
}

impl BaseGenerateContentConfig for ProviderConfig {
    type Error = FixtureError;

    fn supported_optional_params(&self, _model: &str) -> Vec<String> {
        Vec::new()
    }

    fn map_optional_params(
        &self,
        config: &GenerateContentConfigDict,
        _model: &str,
    ) -> GenerateContentConfigDict {
        config.clone()
    }

    fn validate_environment(
        &self,
        request: &EnvironmentRequest<'_>,
    ) -> Result<ResponseHeaders, FixtureError> {
        Ok(request.headers.clone())
    }

    fn transform_request(
        &self,
        _request: &GenerateContentRequest<'_>,
    ) -> Result<GenerateContentRequestDict, FixtureError> {
        Err(FixtureError::Unused)
    }

    fn transform_response(
        &self,
        _request: &ResponseTransform<'_>,
    ) -> Result<GenerateContentResponse, FixtureError> {
        Err(FixtureError::Unused)
    }

    fn auth_token_and_url(
        &self,
        _request: &AuthTokenRequest<'_>,
    ) -> Result<(RequestHeaders, String), FixtureError> {
        Err(FixtureError::Unused)
    }
}
