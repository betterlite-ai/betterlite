use std::collections::BTreeMap;

use betterlite_bedrock::vector_stores::config::{capabilities, validate_environment};
use betterlite_bedrock::vector_stores::endpoint::{CompleteUrl, complete_url};
use betterlite_bedrock::vector_stores::error::VectorStoreError;
use betterlite_bedrock::vector_stores::filters::{
    map_and_or_filters, map_operator_filter, map_operator_to_aws,
};
use betterlite_bedrock::vector_stores::metadata::{file_id_from_metadata, filename_from_metadata};
use betterlite_bedrock::vector_stores::request::user_context;
use betterlite_models::decisions::Json;
use serde_json::{Value, json};

use super::responses::{
    DecodeShape, decode_responses, option_mapping, retrieval_configuration, unsupported_create,
};
use super::{Input, Operation, UrlScenario};
use crate::helpers::HelperError;

pub(super) fn run(input: &Input) -> Value {
    match input.operation {
        Operation::FilterOperatorMap => filter_operator_map(input),
        Operation::NestedAndOrFilters => {
            json!({"filters": mapped_filters(input, map_and_or_filters)})
        }
        Operation::KnowledgeBaseRetrieveUrl => knowledge_base_retrieve_url(input),
        Operation::UserContext => user_context_outcome(input),
        Operation::LocationFileIdName => location_file_id_name(input),
        Operation::AttributesAndScores => decode_responses(input, DecodeShape::Full),
        Operation::UnsupportedCreate => unsupported_create(),
        Operation::OptionMapping => option_mapping(input),
        Operation::RetrievalConfiguration => retrieval_configuration(input),
        Operation::KnowledgeBaseResponseError => decode_responses(input, DecodeShape::Summary),
    }
}

fn filter_operator_map(input: &Input) -> Value {
    let operators = input
        .operators
        .iter()
        .map(|operator| json!(map_operator_to_aws(operator)))
        .collect::<Vec<_>>();
    json!({"operators": operators, "filters": mapped_filters(input, map_operator_filter)})
}

fn mapped_filters(
    input: &Input,
    mapper: fn(&Json) -> Result<Json, VectorStoreError>,
) -> Vec<Value> {
    input
        .filters
        .iter()
        .map(|filter| {
            mapper(filter).map_or_else(
                |_error| json!({"failed": true}),
                |value| json!({"value": value}),
            )
        })
        .collect()
}

fn knowledge_base_retrieve_url(input: &Input) -> Value {
    let urls = input.urls.iter().map(url_outcome).collect::<Vec<_>>();
    let capabilities = capabilities();
    json!({
        "endpoints": capabilities.endpoints,
        "credentials": capabilities.credentials,
        "supported": capabilities.supported_params,
        "headers": validate_environment(
            input.headers.iter().cloned().collect::<BTreeMap<_, _>>(),
        ),
        "urls": urls,
    })
}

fn url_outcome(scenario: &UrlScenario) -> Value {
    let configured = scenario
        .litellm_params
        .get("aws_bedrock_runtime_endpoint")
        .and_then(json_text);
    let region = scenario
        .litellm_params
        .get("aws_region_name")
        .and_then(json_text);
    let complete = complete_url(&CompleteUrl {
        api_base: scenario.api_base.as_deref(),
        configured_endpoint: configured,
        environment_endpoint: scenario
            .environment
            .get("AWS_BEDROCK_RUNTIME_ENDPOINT")
            .map(String::as_str),
        aws_region_name: region,
        environment_region_name: scenario
            .environment
            .get("AWS_REGION_NAME")
            .map(String::as_str),
        environment_region: scenario.environment.get("AWS_REGION").map(String::as_str),
    });
    complete.map_or_else(|_error| json!({"failed": true}), |url| json!({"url": url}))
}

fn json_text(value: &Json) -> Option<&str> {
    match value {
        Json::Text(text) => Some(text),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

fn user_context_outcome(input: &Input) -> Value {
    let selected = input
        .contexts
        .iter()
        .map(|scenario| {
            let extra = match &scenario.extra_body {
                Some(Json::Object(fields)) => Some(fields),
                _ => None,
            };
            user_context(extra, &scenario.litellm_params)
        })
        .collect::<Vec<_>>();
    json!({"selected": selected})
}

fn location_file_id_name(input: &Input) -> Value {
    let values = input
        .metadata
        .iter()
        .map(|entry| {
            let metadata = entry.clone().unwrap_or_default();
            json!({
                "file_id": file_id_from_metadata(&metadata),
                "filename": filename_from_metadata(&metadata),
                "attributes": Json::Object(metadata),
            })
        })
        .collect::<Vec<_>>();
    json!({"values": values})
}

pub(super) fn message_failure(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "FixtureError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
