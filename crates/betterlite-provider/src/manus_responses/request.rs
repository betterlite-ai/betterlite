use betterlite_http::url_path::encode_segment;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::response_input::transform_responses_request;

use crate::api::responses_provider_contract::records::{
    CreateRequest, IdRequest, Params, PreparedRequest,
};

use super::error::ManusResponsesError;
use super::json;
use super::{AGENT_PROFILE_FIELD, TASK_MODE};

const TASK_MODE_FIELD: &str = "task_mode";
const RESPONSE_ID_FIELD: &str = "response_id";

pub(super) fn agent_profile(model: &str) -> &str {
    match model.split_once('/') {
        Some((_provider, profile)) => profile,
        None => model,
    }
}

pub(super) fn create_body(request: &CreateRequest) -> Result<Params, ManusResponsesError> {
    let input = json::responses_input(&request.input)?;
    let options = json::to_prompts_object(&request.optional_params);
    let base =
        transform_responses_request(&request.model, &input, &options, ProviderId::Manus, None)?;
    let mut body = json::from_prompts_object(&base);
    body.insert(
        String::from(TASK_MODE_FIELD),
        Json::Text(String::from(TASK_MODE)),
    );
    body.insert(
        String::from(AGENT_PROFILE_FIELD),
        Json::Text(String::from(agent_profile(&request.model))),
    );
    if let Some(extra) = json::extra_body(&request.optional_params)? {
        body.extend(extra);
    }
    Ok(body)
}

pub(super) fn get_request(request: &IdRequest) -> Result<PreparedRequest, ManusResponsesError> {
    let encoded = encode_segment(&request.response_id, RESPONSE_ID_FIELD).map_err(|source| {
        ManusResponsesError::ResponseId {
            response_id: request.response_id.clone(),
            source,
        }
    })?;
    Ok(PreparedRequest {
        url: format!("{}/{encoded}", request.api_base),
        params: Params::new(),
    })
}
