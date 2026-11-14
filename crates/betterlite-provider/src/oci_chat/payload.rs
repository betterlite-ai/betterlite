use betterlite_models::model_capability_predicates::CapabilityPredicates;
use betterlite_openai::chat::{ChatMessage, Role};
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::oci_chat::config::get_vendor_from_model;
use crate::oci_chat::payload::params::get_optional_params;
use crate::oci_chat_wire::cohere::request::{adapt_cohere_messages, extract_text_content};
use crate::oci_chat_wire::generic::request::adapt_generic_messages;
use crate::oci_chat_wire::types::cohere::CohereChatRequest;
use crate::oci_chat_wire::types::oci::{OciChatRequestPayload, OciVendor};
use crate::oci_common::credentials::OciCredentials;
use crate::oci_common::{OciError, OciServingMode};

mod format;
mod params;

const MISSING_COMPARTMENT: &str = "oci_compartment_id is required for OCI chat requests. Pass it as optional_params or set the OCI_COMPARTMENT_ID env var.";

const BAD_SERVING_MODE: &str = "kwarg `oci_serving_mode` must be either 'ON_DEMAND' or 'DEDICATED'";

const BAD_ENDPOINT_ID: &str = "kwarg `oci_endpoint_id` must be a string";

const NO_USER_MESSAGE: &str = "No user message found: Cohere models need at least one user message";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OciCompletionPayload {
    pub compartment_id: String,
    pub serving_mode: OciServingMode,
    pub chat_request: OciChatRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum OciChatRequest {
    Cohere(CohereChatRequest),
    Generic(OciChatRequestPayload),
}

pub async fn transform_request<Predicates>(
    model: &str,
    messages: &[ChatMessage],
    optional_params: &Object,
    credentials: &OciCredentials,
    predicates: &mut Predicates,
) -> Result<OciCompletionPayload, OciError>
where
    Predicates: CapabilityPredicates + Send,
{
    let Some(compartment_id) = credentials
        .oci_compartment_id
        .as_deref()
        .filter(|value| !value.is_empty())
    else {
        return Err(OciError {
            status: 400,
            message: MISSING_COMPARTMENT.to_owned(),
        });
    };
    let vendor = get_vendor_from_model(model);
    let serving_mode = serving_mode(model, optional_params)?;
    let selected = get_optional_params(vendor, optional_params, model, predicates).await?;
    let chat_request = match vendor {
        OciVendor::Cohere => OciChatRequest::Cohere(cohere_request(messages, &selected)?),
        OciVendor::Generic => OciChatRequest::Generic(generic_request(messages, &selected)?),
    };
    Ok(OciCompletionPayload {
        compartment_id: compartment_id.to_owned(),
        serving_mode,
        chat_request,
    })
}

fn serving_mode(model: &str, optional_params: &Object) -> Result<OciServingMode, OciError> {
    match text_param(
        optional_params,
        "oci_serving_mode",
        "ON_DEMAND",
        BAD_SERVING_MODE,
    )?
    .as_str()
    {
        "ON_DEMAND" => Ok(OciServingMode::OnDemand {
            model_id: model.to_owned(),
        }),
        "DEDICATED" => Ok(OciServingMode::Dedicated {
            endpoint_id: text_param(optional_params, "oci_endpoint_id", model, BAD_ENDPOINT_ID)?,
        }),
        _ => Err(OciError {
            status: 400,
            message: BAD_SERVING_MODE.to_owned(),
        }),
    }
}

fn text_param(
    optional_params: &Object,
    key: &str,
    default: &str,
    message: &str,
) -> Result<String, OciError> {
    match optional_params.get(key) {
        None | Some(Json::Null) => Ok(default.to_owned()),
        Some(Json::Text(text)) => Ok(text.clone()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => Err(OciError {
            status: 400,
            message: message.to_owned(),
        }),
    }
}

fn cohere_request(
    messages: &[ChatMessage],
    selected: &Object,
) -> Result<CohereChatRequest, OciError> {
    let Some(last_user) = messages
        .iter()
        .rev()
        .find(|message| message.role == Role::User)
    else {
        return Err(OciError {
            status: 400,
            message: NO_USER_MESSAGE.to_owned(),
        });
    };
    let preamble = messages
        .iter()
        .filter(|message| message.role == Role::System)
        .map(|message| extract_text_content(message.content.as_ref()))
        .collect::<Vec<String>>()
        .join("\n");
    let history: Vec<ChatMessage> = messages
        .iter()
        .filter(|message| message.role != Role::System)
        .cloned()
        .collect();
    let mut fields = selected.clone();
    fields.insert(
        String::from("apiFormat"),
        Json::Text(OciVendor::Cohere.api_format().to_owned()),
    );
    fields.insert(
        String::from("message"),
        Json::Text(extract_text_content(last_user.content.as_ref())),
    );
    fields.insert(
        String::from("chatHistory"),
        typed(&adapt_cohere_messages(&history), "Json")?,
    );
    if !preamble.is_empty() {
        fields.insert(String::from("preambleOverride"), Json::Text(preamble));
    }
    typed(&Json::Object(fields), "CohereChatRequest")
}

fn generic_request(
    messages: &[ChatMessage],
    selected: &Object,
) -> Result<OciChatRequestPayload, OciError> {
    let mut fields = selected.clone();
    fields.insert(
        String::from("apiFormat"),
        Json::Text(OciVendor::Generic.api_format().to_owned()),
    );
    fields.insert(
        String::from("messages"),
        typed(&adapt_generic_messages(messages)?, "Json")?,
    );
    typed(&Json::Object(fields), "OCIChatRequestPayload")
}

fn typed<S, T>(value: &S, type_name: &str) -> Result<T, OciError>
where
    S: Serialize,
    T: DeserializeOwned,
{
    let bytes = serde_json::to_vec(value).map_err(|source| encode_error(&source))?;
    serde_json::from_slice(&bytes).map_err(|source| OciError {
        status: 400,
        message: format!("the OCI chat request does not fit {type_name}: {source}"),
    })
}

pub(crate) fn encode_error(source: &serde_json::Error) -> OciError {
    OciError {
        status: 500,
        message: format!("the OCI chat request cannot be encoded as JSON: {source}"),
    }
}
