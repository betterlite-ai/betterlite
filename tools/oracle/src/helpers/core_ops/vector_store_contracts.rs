use std::collections::BTreeMap;

use betterlite_core::api::vector_store_contracts::filters::{
    requested_create_params, requested_search_params,
};
use betterlite_core::api::vector_store_contracts::metadata::{
    metadata_map, vector_store_request_metadata,
};
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::{
    VectorStoreCreateResponse, VectorStoreSearchResponse,
};
use betterlite_provider::api::vector_store_contracts::config::{
    AuthCredentials, EndpointsByType, Params, PreparedRequest, SearchQuery, SearchRequest,
    SignRequest, VectorStoreConfig, VectorStoreLogging, VectorStoreResponse,
};
use betterlite_provider::api::vector_store_contracts::error::{
    BaseLlmException, VectorStoreContractError,
};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Number, Value, json};
use thiserror::Error;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    OptionalParameterFilters {
        create: Vec<Params>,
        search: Vec<Params>,
    },
    RequestMetadataPrecedence {
        cases: Vec<MetadataCase>,
    },
    BaseContractDefaults(Box<DefaultsInput>),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MetadataCase {
    #[serde(default)]
    litellm_metadata: Option<Slot>,
    #[serde(default)]
    metadata: Option<Slot>,
}

// a Python caller can build a dict with keys that are not strings, which JSON objects cannot carry
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Slot {
    Entries { entries: Vec<(Value, Value)> },
    Value { value: Value },
}

enum SlotValue {
    Absent,
    Present(Json),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DefaultsInput {
    optional_params: Params,
    non_default_params: Params,
    headers: BTreeMap<String, String>,
    request_data: Params,
    api_key: Option<String>,
    api_base: String,
    api_bases: Vec<Option<String>>,
    error: ErrorInput,
    forward: ForwardInput,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorInput {
    message: String,
    status_code: u16,
    headers: Params,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ForwardInput {
    vector_store_id: String,
    query: SearchQuery,
    optional_params: Params,
    api_base: String,
    litellm_params: Params,
    extra_body: Option<Params>,
}

pub(crate) struct Op;

#[derive(Debug, Error)]
enum FixtureError {
    #[error(transparent)]
    Contract(#[from] VectorStoreContractError),
}

struct MinimalConfig;

struct RecordingConfig;

impl VectorStoreConfig for MinimalConfig {
    type Error = FixtureError;

    fn auth_credentials(&self, _litellm_params: &Params) -> AuthCredentials {
        AuthCredentials::default()
    }

    fn endpoints_by_type(&self) -> EndpointsByType {
        EndpointsByType::default()
    }

    fn transform_search_request(
        &self,
        request: SearchRequest<'_>,
    ) -> Result<PreparedRequest, Self::Error> {
        let calls = match request.logging.model_call_details.get("calls") {
            Some(Json::Number(number)) => number.as_u64().unwrap_or_default(),
            _ => 0,
        };
        request.logging.model_call_details.insert(
            "calls".to_owned(),
            Json::Number(Number::from(calls.saturating_add(1))),
        );
        let query = match request.query {
            SearchQuery::Text(text) => Json::Text(text.clone()),
            SearchQuery::Sequence(texts) => {
                Json::List(texts.iter().map(|text| Json::Text(text.clone())).collect())
            }
        };
        let body = Params::from([
            (
                "vector_store_id".to_owned(),
                Json::Text(request.vector_store_id.to_owned()),
            ),
            ("query".to_owned(), query),
            (
                "optional_params".to_owned(),
                object(request.optional_params),
            ),
            (
                "api_base".to_owned(),
                Json::Text(request.api_base.to_owned()),
            ),
            ("litellm_params".to_owned(), object(request.litellm_params)),
            (
                "extra_body".to_owned(),
                request.extra_body.map_or(Json::Null, object),
            ),
        ]);
        Ok((format!("{}/vector_stores/search", request.api_base), body))
    }

    fn transform_search_response(
        &self,
        _response: &VectorStoreResponse,
        _logging: &mut VectorStoreLogging,
    ) -> Result<VectorStoreSearchResponse, Self::Error> {
        Err(FixtureError::Contract(
            VectorStoreContractError::ApiBaseRequired,
        ))
    }

    fn transform_create_request(
        &self,
        _optional_params: &Params,
        _api_base: &str,
    ) -> Result<PreparedRequest, Self::Error> {
        Err(FixtureError::Contract(
            VectorStoreContractError::ApiBaseRequired,
        ))
    }

    fn transform_create_response(
        &self,
        _response: &VectorStoreResponse,
    ) -> Result<VectorStoreCreateResponse, Self::Error> {
        Err(FixtureError::Contract(
            VectorStoreContractError::ApiBaseRequired,
        ))
    }
}

impl VectorStoreConfig for RecordingConfig {
    type Error = FixtureError;

    fn auth_credentials(&self, _litellm_params: &Params) -> AuthCredentials {
        AuthCredentials::default()
    }

    fn endpoints_by_type(&self) -> EndpointsByType {
        EndpointsByType::default()
    }

    fn map_params(
        &self,
        non_default_params: &Params,
        mut optional_params: Params,
        drop_params: bool,
    ) -> Params {
        let observed = Params::from([
            ("non_default_params".to_owned(), object(non_default_params)),
            ("drop_params".to_owned(), Json::Bool(drop_params)),
        ]);
        optional_params.insert(
            "observed".to_owned(),
            Json::Object(observed.into_iter().collect()),
        );
        optional_params
    }

    fn transform_search_request(
        &self,
        _request: SearchRequest<'_>,
    ) -> Result<PreparedRequest, Self::Error> {
        Err(FixtureError::Contract(
            VectorStoreContractError::ApiBaseRequired,
        ))
    }

    fn transform_search_response(
        &self,
        _response: &VectorStoreResponse,
        _logging: &mut VectorStoreLogging,
    ) -> Result<VectorStoreSearchResponse, Self::Error> {
        Err(FixtureError::Contract(
            VectorStoreContractError::ApiBaseRequired,
        ))
    }

    fn transform_create_request(
        &self,
        _optional_params: &Params,
        _api_base: &str,
    ) -> Result<PreparedRequest, Self::Error> {
        Err(FixtureError::Contract(
            VectorStoreContractError::ApiBaseRequired,
        ))
    }

    fn transform_create_response(
        &self,
        _response: &VectorStoreResponse,
    ) -> Result<VectorStoreCreateResponse, Self::Error> {
        Err(FixtureError::Contract(
            VectorStoreContractError::ApiBaseRequired,
        ))
    }
}

fn object(params: &Params) -> Json {
    Json::Object(
        params
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
    )
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::OptionalParameterFilters { create, search } => {
                Ok(optional_filters(create, search))
            }
            Input::RequestMetadataPrecedence { cases } => {
                Ok(Value::Array(cases.iter().map(metadata_case).collect()))
            }
            Input::BaseContractDefaults(defaults) => defaults_report(defaults),
        }
    }
}

fn optional_filters(create: &[Params], search: &[Params]) -> Value {
    let filtered_create: Vec<Params> = create.iter().map(requested_create_params).collect();
    let filtered_search: Vec<Value> = search
        .iter()
        .map(|params| {
            json!({
                "default": requested_search_params(params, &MinimalConfig),
                "mapped": requested_search_params(params, &RecordingConfig),
            })
        })
        .collect();
    json!({"create": filtered_create, "search": filtered_search})
}

fn metadata_case(case: &MetadataCase) -> Value {
    let Some((kwargs, result)) = described_case(case) else {
        return json!({"ok": false, "class": "ValidationError"});
    };
    json!({"ok": true, "kwargs": kwargs, "result": result})
}

fn described_case(case: &MetadataCase) -> Option<(Params, Params)> {
    let mut kwargs = Params::new();
    match slot_value(case.litellm_metadata.as_ref(), "litellm_metadata")? {
        SlotValue::Absent => {}
        SlotValue::Present(value) => {
            kwargs.insert("litellm_metadata".to_owned(), value);
        }
    }
    match slot_value(case.metadata.as_ref(), "metadata")? {
        SlotValue::Absent => {}
        SlotValue::Present(value) => {
            kwargs.insert("metadata".to_owned(), value);
        }
    }
    let result = vector_store_request_metadata(&kwargs);
    Some((kwargs, result))
}

fn slot_value(slot: Option<&Slot>, field: &'static str) -> Option<SlotValue> {
    match slot {
        None => Some(SlotValue::Absent),
        Some(Slot::Value { value }) => Some(SlotValue::Present(
            serde_json::from_value(value.clone()).ok()?,
        )),
        Some(Slot::Entries { entries }) => {
            let converted: Vec<(Json, Json)> = entries
                .iter()
                .map(|(key, value)| {
                    Some((
                        serde_json::from_value(key.clone()).ok()?,
                        serde_json::from_value(value.clone()).ok()?,
                    ))
                })
                .collect::<Option<_>>()?;
            let built = metadata_map(converted, field).ok()?;
            Some(SlotValue::Present(Json::Object(
                built.into_iter().collect(),
            )))
        }
    }
}

fn defaults_report(input: &DefaultsInput) -> crate::helpers::HelperResult {
    let api_key = input.api_key.as_deref().map(SecretString::from);
    let (signed_headers, signed_body) = MinimalConfig.sign_request(SignRequest {
        headers: &input.headers,
        optional_params: &input.optional_params,
        request_data: &input.request_data,
        api_base: &input.api_base,
        api_key: api_key.as_ref(),
    });
    let urls: Vec<Value> = input
        .api_bases
        .iter()
        .map(|api_base| complete_url(api_base.as_deref()))
        .collect();
    let raised: BaseLlmException = MinimalConfig.error_class(
        &input.error.message,
        input.error.status_code,
        input.error.headers.clone(),
    );
    let response = VectorStoreSearchResponse::default();
    let (prompt, completion) = MinimalConfig.calculate_vector_store_cost(&response);
    let sync = forward(&input.forward, false)?;
    let asynchronous = forward(&input.forward, true)?;
    Ok(json!({
        "supported": MinimalConfig.supported_params("openai/text-embedding-3-small"),
        "mapped": MinimalConfig.map_params(&input.non_default_params, input.optional_params.clone(), false),
        "environment": MinimalConfig.validate_environment(&input.headers, None),
        "urls": urls,
        "signed_headers": signed_headers,
        "signed_body": signed_body,
        "cost": [decimal_number(prompt), decimal_number(completion)],
        "error": {
            "status_code": raised.status_code,
            "message": raised.message,
            "headers": raised.headers,
        },
        "sync": sync,
        "async": asynchronous,
    }))
}

fn complete_url(api_base: Option<&str>) -> Value {
    match MinimalConfig.complete_url(api_base, &Params::new()) {
        Ok(url) => Value::String(url),
        Err(error) => json!({"error": {"class": "ValueError", "message": error.to_string()}}),
    }
}

fn decimal_number(decimal: rust_decimal::Decimal) -> Value {
    let mut text = decimal.normalize().to_string();
    if !text.contains('.') && !text.contains('e') {
        text.push_str(".0");
    }
    text.parse::<Number>().map_or(Value::Null, Value::Number)
}

fn forward(input: &ForwardInput, asynchronous: bool) -> crate::helpers::HelperResult {
    let mut logging = VectorStoreLogging::default();
    let request = SearchRequest {
        vector_store_id: &input.vector_store_id,
        query: &input.query,
        optional_params: &input.optional_params,
        api_base: &input.api_base,
        logging: &mut logging,
        litellm_params: &input.litellm_params,
        extra_body: input.extra_body.as_ref(),
    };
    let pair = if asynchronous {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| fixture_error(error.to_string()))?;
        runtime
            .block_on(MinimalConfig.transform_search_request_async(request))
            .map_err(|error| fixture_error(error.to_string()))?
    } else {
        MinimalConfig
            .transform_search_request(request)
            .map_err(|error| fixture_error(error.to_string()))?
    };
    Ok(json!({
        "url": pair.0,
        "body": pair.1,
        "logging": {"calls": logging.model_call_details.get("calls")},
    }))
}

fn fixture_error(message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: String::from("ValueError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
