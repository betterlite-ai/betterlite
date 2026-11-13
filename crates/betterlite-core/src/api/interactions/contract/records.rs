use std::collections::BTreeMap;

use betterlite_interactions::types::interaction::InteractionInput;
use betterlite_models::controls::json_value::JsonValue;
use secrecy::SecretString;

pub type Params = BTreeMap<String, JsonValue>;
pub type RequestHeaders = BTreeMap<String, SecretString>;
pub type ResponseHeaders = BTreeMap<String, String>;

pub struct EnvironmentRequest<'request> {
    pub headers: &'request RequestHeaders,
    pub model: &'request str,
    pub litellm_params: Option<&'request Params>,
}

pub struct CompleteUrlRequest<'request> {
    pub api_base: Option<&'request str>,
    pub model: Option<&'request str>,
    pub agent: Option<&'request str>,
    pub litellm_params: Option<&'request Params>,
    pub stream: Option<bool>,
}

pub struct CreateInteractionRequest<'request> {
    pub model: Option<&'request str>,
    pub agent: Option<&'request str>,
    pub input: Option<&'request InteractionInput>,
    pub optional_params: &'request Params,
    pub litellm_params: &'request Params,
    pub headers: &'request RequestHeaders,
}

pub struct InteractionIdRequest<'request> {
    pub interaction_id: &'request str,
    pub api_base: &'request str,
    pub litellm_params: &'request Params,
    pub headers: &'request RequestHeaders,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PreparedRequest {
    pub url: String,
    // LiteLLM's lifecycle transforms answer query params for get and a request body for delete and cancel
    pub params: Params,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InteractionsUpstream {
    pub status_code: u16,
    pub headers: ResponseHeaders,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct InteractionsLogging {
    pub model_call_details: Params,
}
