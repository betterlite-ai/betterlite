use betterlite_openai::api::rerank_contracts::records::RerankResponse;
use betterlite_provider::api::rerank_contracts::{
    BaseRerankConfig as _, Headers, Params, RerankLogging, RerankProviderResponse,
    ResponseTransform,
};
use serde_json::{Value, json};

use super::recording::{Recording, object_of};
use super::wire_records::decode_response;
use super::{ContractError, DefaultsParams};

pub(super) fn run(params: &DefaultsParams) -> Result<Value, ContractError> {
    let config = Recording::default();
    let urls = params
        .api_bases
        .iter()
        .map(|api_base| config.get_complete_url(api_base.as_deref(), "rerank-model", None))
        .collect::<Vec<_>>();
    let request =
        config.transform_rerank_request("rerank-model", &Params::new(), &Headers::new(), None)?;
    let raised = config.get_error_class(
        &params.error.message,
        params.error.status_code,
        params.error.headers.clone(),
    );
    Ok(json!({
        "urls": urls,
        "request": object_of(&request),
        "response": default_response(&config, &params.response)?,
        "error": {
            "status_code": raised.status_code,
            "message": raised.message,
            "headers": raised.headers,
        },
    }))
}

fn default_response(config: &Recording, raw: &Value) -> Result<Value, ContractError> {
    let response: RerankResponse = decode_response(raw).ok_or(ContractError::Response)?;
    let mut logging = RerankLogging::default();
    let raw_response = RerankProviderResponse {
        status_code: 200,
        headers: Headers::new(),
        body: Vec::new(),
    };
    let request_data = Params::new();
    let optional_params = Params::new();
    let litellm_params = Params::new();
    let transformed = config.transform_rerank_response(
        response,
        ResponseTransform {
            model: "rerank-model",
            raw_response: &raw_response,
            logging: &mut logging,
            api_key: None,
            request_data: &request_data,
            optional_params: &optional_params,
            litellm_params: &litellm_params,
        },
    )?;
    Ok(serde_json::to_value(transformed).unwrap_or(Value::Null))
}
