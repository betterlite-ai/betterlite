use std::collections::BTreeMap;
use std::str::FromStr as _;

use betterlite_anthropic::api::messages_errors::wire::ResponseRecord;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json as HiddenJson;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::json_provider_registry::{
    BaseClass, Constraints, ProviderConfig, ProviderDescriptor,
};
use betterlite_openai::edenai_common::{
    EDENAI_API_BASE, EDENAI_COST_HEADER, EdenAiEnv, MissingApiKey, cost_value, require_api_key,
};
use betterlite_prompts::arguments::json::{Json, Object};
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde_json::{Number, Value};
use thiserror::Error;

use crate::api::anthropic_beta_filter::Headers;
use crate::api::api_boundaries::ApiLimits;
use crate::api::messages_thinking::response::{ResponseJsonError, decode_messages_response};
use crate::json_messages_policy::{
    JsonMessagesPolicy, JsonMessagesPolicyError, MessagesCredentials,
};

mod tests;

pub const API_KEY_ENV: &str = "EDENAI_API_KEY";
pub const API_BASE_ENV: &str = "EDENAI_API_BASE";
pub const BASE_CLASS: &str = "openai_gpt";

const COST: &str = "cost";
const RESPONSE_COST: &str = "response_cost";

// LiteLLM's _EDENAI_PROVIDER_SPEC, with no constraints and the plain OpenAI base class
#[must_use]
pub fn descriptor() -> ProviderDescriptor {
    ProviderDescriptor {
        provider: ProviderId::Edenai,
        config: ProviderConfig {
            base_url: String::from(EDENAI_API_BASE),
            api_key_env: String::from(API_KEY_ENV),
            api_base_env: Some(String::from(API_BASE_ENV)),
            base_class: BaseClass(String::from(BASE_CLASS)),
            param_mappings: BTreeMap::new(),
            constraints: Constraints::default(),
            special_handling: BTreeMap::new(),
            supported_endpoints: Vec::new(),
        },
    }
}

#[derive(Debug, Error)]
pub enum EdenAiMessagesError {
    #[error(transparent)]
    MissingKey(#[from] MissingApiKey),
    #[error(transparent)]
    Policy(#[from] JsonMessagesPolicyError),
    #[error(transparent)]
    Response(#[from] ResponseJsonError),
}

impl EdenAiMessagesError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::MissingKey(error) => Some(error.status_code),
            Self::Policy(
                JsonMessagesPolicyError::BaseRequired | JsonMessagesPolicyError::Beta(_),
            ) => None,
            Self::Policy(JsonMessagesPolicyError::Transform(error)) => Some(error.status_code()),
            Self::Response(error) => Some(error.status_code()),
        }
    }

    #[must_use]
    pub const fn class_name(&self) -> &'static str {
        match self {
            Self::MissingKey(_) => "AuthenticationError",
            Self::Policy(JsonMessagesPolicyError::BaseRequired) => "BadRequestError",
            Self::Policy(JsonMessagesPolicyError::Beta(_)) => "BetaFilterError",
            Self::Policy(JsonMessagesPolicyError::Transform(_)) => "AnthropicError",
            Self::Response(_) => "AnthropicError",
        }
    }
}

pub struct EnvironmentInput<'call> {
    pub headers: Headers,
    pub api_key: Option<&'call SecretString>,
    pub model: &'call str,
    pub env: &'call EdenAiEnv,
    pub lookup: &'call dyn EnvironmentLookup,
    pub optional_params: &'call Object,
    pub messages: &'call [Json],
}

// Eden's key check runs first, so a missing key never reaches the policy's own resolution
pub fn validate_environment(
    policy: &JsonMessagesPolicy,
    input: EnvironmentInput<'_>,
) -> Result<Headers, EdenAiMessagesError> {
    let key = require_api_key(input.api_key, input.model, input.env)?;
    let credentials = MessagesCredentials {
        api_base: None,
        api_key: Some(&key),
        global_api_key: None,
        lookup: input.lookup,
    };
    let headers = policy.validate_environment(
        input.headers,
        &credentials,
        input.optional_params,
        input.messages,
    )?;
    Ok(headers)
}

pub struct ReplyInput<'request> {
    pub body: &'request [u8],
    pub status: u16,
    pub limits: &'request ApiLimits,
    pub state: &'request mut LoggingState,
}

// the reply as sent, then the cost it carries goes to the call log and a reply without one leaves it alone
pub fn transform_response(input: ReplyInput<'_>) -> Result<ResponseRecord, EdenAiMessagesError> {
    let record = decode_messages_response(input.body, input.status, input.limits)?;
    if let Some(cost) = reply_cost(&record)
        && let Some(number) = Number::from_str(&cost.normalize().to_string()).ok()
    {
        input
            .state
            .model_call_details
            .insert(String::from(RESPONSE_COST), HiddenJson::Number(number));
    }
    Ok(record)
}

// pydantic's AliasChoices reads the first alias present, and an invalid value there wins over the other alias
fn reply_cost(record: &ResponseRecord) -> Option<Decimal> {
    let raw = record
        .fields
        .get(COST)
        .or_else(|| record.fields.get(EDENAI_COST_HEADER))?;
    let value: Value = serde_json::from_str(raw.get()).ok()?;
    cost_value(&value)
}
