use std::collections::BTreeMap;

use betterlite_auth::sso_types::identity::ReturnedUiTokenObject;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::ui_session_token::Settings as UiSessionSettings;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::ui_session_token::{
    UiSessionTokenError, disabled_non_admin_personal_key_creation, encode_ui_session_jwt,
};
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

// the token object is boxed to keep the tagged enum's variant sizes within rustc's size-difference lint
#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Encode {
        token: Box<ReturnedUiTokenObject>,
        master_key: String,
        duration: String,
        now: DateTime<Utc>,
    },
    PersonalKeyFlag {
        #[serde(default)]
        key_generation_settings: Option<Value>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Encode {
                token,
                master_key,
                duration,
                now,
            } => encode(token, master_key, duration, *now),
            Input::PersonalKeyFlag {
                key_generation_settings,
            } => personal_key_flag(key_generation_settings.as_ref()),
        }
    }
}

fn encode(
    token: &ReturnedUiTokenObject,
    master_key: &str,
    duration: &str,
    now: DateTime<Utc>,
) -> HelperResult {
    let settings = UiSessionSettings {
        duration: duration.to_owned(),
    };
    let database = TimeZoneDatabase::from_env();
    let secret = SecretString::from(master_key.to_owned());
    match encode_ui_session_jwt(token, &secret, &settings, now, &database) {
        Ok(encoded) => decode_token(&encoded, master_key),
        Err(error) => Ok(Value::String(error_class(&error).to_owned())),
    }
}

fn decode_token(encoded: &str, master_key: &str) -> HelperResult {
    let header = decode_header(encoded).or_value_error()?;
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = false;
    let decoding_key = DecodingKey::from_secret(master_key.as_bytes());
    let claims = decode::<Value>(encoded, &decoding_key, &validation)
        .or_value_error()?
        .claims;
    Ok(json!({"header": header, "claims": claims}))
}

// the class names PyJWT raises for the inputs this helper sends
const fn error_class(error: &UiSessionTokenError) -> &'static str {
    match error {
        UiSessionTokenError::Duration(_) => "ValueError",
        UiSessionTokenError::Overflow => "OverflowError",
        UiSessionTokenError::EmptyMasterKey | UiSessionTokenError::Encode(_) => "InvalidKeyError",
    }
}

fn personal_key_flag(settings: Option<&Value>) -> HelperResult {
    // an absent setting and a null one both leave litellm.key_generation_settings at None
    let configured = settings.cloned().unwrap_or(Value::Null);
    let source = json!({"litellm_settings": {"key_generation_settings": configured}}).to_string();
    let lookup = BTreeMap::<String, String>::new();
    let config = ConfigLoader { lookup }.load(&source).or_value_error()?;
    Ok(Value::Bool(disabled_non_admin_personal_key_creation(
        &config.sections.key_access.key_generation,
    )))
}
