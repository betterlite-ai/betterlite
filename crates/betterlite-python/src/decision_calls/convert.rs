use std::collections::BTreeMap;
use std::time::{Duration, TryFromFloatSecsError};

use betterlite_core::api::decisions_send::format::DecisionsOutput;
use betterlite_core::decisions::DecisionsArgs;
use betterlite_core::systemone::SystemOneArgs;
use betterlite_error::error::{ErrorClass, class_message};
use betterlite_models::decisions::{Extra, HiddenParams, Json};
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::openai_base_config::Headers;
use betterlite_sdk::call_wrapper::error::{class_name, class_status};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

mod tests;

// LiteLLM raises these builtins from its sandbox providers; raise_native_error builds neither
pub(crate) const VALUE_ERROR: &str = "ValueError";
pub(crate) const TIMEOUT_ERROR: &str = "TimeoutError";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct SessionWire {
    pub(crate) environment: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct CallWire {
    pub(crate) operation: String,
    pub(crate) settings: SettingsWire,
    // nested, not flattened: arbitrary_precision turns a flattened float into a map
    pub(crate) arguments: Arguments,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct SettingsWire {
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) openai_key: Option<SecretString>,
    pub(crate) organization: Option<String>,
    pub(crate) drop_params: Option<bool>,
}

impl SettingsWire {
    pub(crate) fn llm_globals(&self) -> LiteLlmGlobals {
        LiteLlmGlobals {
            api_base: self.api_base.clone(),
            api_key: self.api_key.clone(),
            openai_key: self.openai_key.clone(),
            organization: self.organization.clone(),
        }
    }

    pub(crate) fn global_credentials(&self) -> GlobalCredentials {
        GlobalCredentials {
            api_key: self.api_key.clone(),
            api_base: self.api_base.clone(),
            xai_key: None,
            inception_key: None,
            lemonade_key: None,
        }
    }

    // ast-grep-ignore: no-one-line-functions - four entrypoints read the wire's drop_params rule through this predicate
    pub(crate) const fn global_drop_params(&self) -> bool {
        matches!(self.drop_params, Some(true))
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Arguments {
    pub(crate) model: String,
    pub(crate) input: Option<Json>,
    pub(crate) state: Option<Json>,
    pub(crate) questions: Option<Json>,
    pub(crate) safety_identifier: Option<String>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) api_base: Option<String>,
    pub(crate) timeout: Option<f64>,
    pub(crate) custom_llm_provider: Option<String>,
    pub(crate) extra_headers: Option<Headers>,
    pub(crate) provider: String,
    pub(crate) container: Json,
    pub(crate) code: String,
    pub(crate) template: Option<String>,
    // the sandbox create timeout is an integer while the decisions timeout is a float
    pub(crate) create_timeout: Option<i64>,
    pub(crate) allow_internet_access: Option<bool>,
    pub(crate) metadata: Option<BTreeMap<String, String>>,
    pub(crate) env_vars: Option<BTreeMap<String, String>>,
    pub(crate) resource_limits: Option<BTreeMap<String, String>>,
    pub(crate) resource_requests: Option<BTreeMap<String, String>>,
    pub(crate) entrypoint: Option<Vec<String>>,
    pub(crate) network_policy: Option<Json>,
    pub(crate) secure_access: Option<bool>,
    pub(crate) use_server_proxy: Option<bool>,
    pub(crate) ready_timeout: Option<f64>,
    pub(crate) poll_interval: Option<f64>,
    pub(crate) language: Option<String>,
    pub(crate) name: Option<String>,
    pub(crate) tools: Vec<Json>,
    pub(crate) environment: Option<BTreeMap<String, String>>,
    pub(crate) params: Extra,
}

macro_rules! decision_arguments {
    ($args:ident, $arguments:expr, { $($own:ident: $value:expr),* $(,)? }) => {
        $args {
            $($own: $value,)*
            model: $arguments.model.clone(),
            questions: $arguments.questions.clone(),
            safety_identifier: $arguments.safety_identifier.clone(),
            api_key: $arguments.api_key.clone(),
            api_base: $arguments.api_base.clone(),
            timeout: requested_timeout($arguments.timeout)?,
            custom_llm_provider: $arguments.custom_llm_provider.clone(),
            extra_headers: $arguments.extra_headers.clone(),
            kwargs: $arguments.params.clone(),
        }
    };
}

impl Arguments {
    pub(crate) fn decisions(&self) -> Result<DecisionsArgs, ConvertError> {
        Ok(decision_arguments!(DecisionsArgs, self, {
            input: self.input.clone(),
        }))
    }

    pub(crate) fn systemone(&self) -> Result<SystemOneArgs, ConvertError> {
        Ok(decision_arguments!(SystemOneArgs, self, {
            state: self.state.clone(),
        }))
    }
}

pub(crate) fn requested_timeout(given: Option<f64>) -> Result<Option<Duration>, ConvertError> {
    let Some(seconds) = given else {
        return Ok(None);
    };
    Duration::try_from_secs_f64(seconds)
        .map(Some)
        .map_err(|source| ConvertError::Timeout { seconds, source })
}

#[derive(Debug, Serialize)]
pub(crate) struct Answer<'answer, T> {
    #[serde(flatten)]
    pub(crate) response: &'answer T,
    #[serde(rename = "_hidden_params")]
    pub(crate) hidden_params: &'answer HiddenParams,
}

pub(crate) fn response_answer<'answer, T>(
    response: &'answer T,
    hidden_params: &'answer HiddenParams,
) -> Result<String, ConvertError>
where
    T: Serialize,
{
    encode(&Answer {
        response,
        hidden_params,
    })
}

// Either request format answers a response object whose own type the caller reads.
pub(crate) fn decisions_answer(output: &DecisionsOutput) -> Result<String, ConvertError> {
    match output {
        DecisionsOutput::SystemOne(response) => response_answer(response, &response.hidden_params),
        DecisionsOutput::OpenAi(response) => response_answer(response, &response.hidden_params),
    }
}

// ast-grep-ignore: no-one-line-functions - every answer passes through this one JSON encoder, which maps serde's error once
pub(crate) fn encode<T>(value: &T) -> Result<String, ConvertError>
where
    T: Serialize,
{
    serde_json::to_string(value).map_err(|source| ConvertError::Encode { source })
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct NativeDecisionPayload {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) message: String,
    pub(crate) status_code: Option<u16>,
    pub(crate) llm_provider: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) body: Option<Json>,
    pub(crate) headers: BTreeMap<String, String>,
}

impl NativeDecisionPayload {
    pub(crate) fn from_class(class: &ErrorClass) -> Self {
        let metadata = class.metadata();
        let mut headers = BTreeMap::new();
        if let ErrorClass::RateLimit(details) = class {
            headers.clone_from(&details.headers);
        }
        Self {
            kind: class_name(class).to_owned(),
            message: class_message(class),
            // a plain Exception carries no status_code; every LiteLLM error class does
            status_code: class_status(class),
            llm_provider: metadata.provider,
            model: metadata.model,
            body: None,
            headers,
        }
    }

    pub(crate) fn builtin(kind: &str, message: String) -> Self {
        Self {
            kind: kind.to_owned(),
            message,
            status_code: None,
            llm_provider: None,
            model: None,
            body: None,
            headers: BTreeMap::new(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ConvertError {
    #[error("the native request text is not JSON of the expected shape: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the answer could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the timeout of {seconds} seconds is not a usable duration")]
    Timeout {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error("the network policy could not be encoded: {source}")]
    Policy {
        #[source]
        source: serde_json::Error,
    },
    #[error("the sandbox container is neither a sandbox id nor a container handle")]
    Container,
    #[error("the {operation} operation is not a decision call")]
    Operation { operation: String },
}

pub(crate) fn parse<'text, T>(text: &'text str) -> Result<T, ConvertError>
where
    T: Deserialize<'text>,
{
    match serde_json::from_str(text) {
        Ok(value) => Ok(value),
        Err(source) => Err(ConvertError::Json { source }),
    }
}
