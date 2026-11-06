use betterlite_config::loader::EnvironmentLookup;
use secrecy::SecretString;

pub const API_KEY_VARIABLE: &str = "OPENMETER_API_KEY";
pub const ENDPOINT_VARIABLE: &str = "OPENMETER_API_ENDPOINT";
pub const EVENT_TYPE_VARIABLE: &str = "OPENMETER_EVENT_TYPE";
pub const TRUST_REQUEST_USER_VARIABLE: &str = "OPENMETER_TRUST_REQUEST_USER";
pub const DEFAULT_ENDPOINT: &str = "https://openmeter.cloud";
pub const DEFAULT_EVENT_TYPE: &str = "litellm_tokens";

#[derive(Debug)]
pub struct OpenMeterSettings {
    pub api_key: SecretString,
    pub endpoint: String,
    pub event_type: String,
    pub trust_request_user: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum OpenMeterSettingsError {
    #[error("OpenMeter logging needs these environment variables: {missing_keys:?}")]
    MissingKeys { missing_keys: Vec<&'static str> },
    #[error(transparent)]
    Environment(#[from] betterlite_env::EnvError),
}

impl OpenMeterSettings {
    pub fn from_environment(
        environment: &dyn EnvironmentLookup,
    ) -> Result<Self, OpenMeterSettingsError> {
        let api_key = environment
            .lookup(API_KEY_VARIABLE)
            .filter(|value| !value.is_empty());
        let Some(api_key_value) = api_key else {
            return Err(OpenMeterSettingsError::MissingKeys {
                missing_keys: vec![API_KEY_VARIABLE],
            });
        };
        let endpoint = environment
            .lookup(ENDPOINT_VARIABLE)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_ENDPOINT.to_owned());
        let event_type = environment
            .lookup(EVENT_TYPE_VARIABLE)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_EVENT_TYPE.to_owned());
        let trust_text = environment
            .lookup(TRUST_REQUEST_USER_VARIABLE)
            .unwrap_or_default();
        let trust_request_user =
            betterlite_env::flag(TRUST_REQUEST_USER_VARIABLE, &trust_text)?.unwrap_or(true);
        Ok(Self {
            api_key: SecretString::from(api_key_value),
            endpoint,
            event_type,
            trust_request_user,
        })
    }
}
