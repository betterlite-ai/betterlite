use std::collections::BTreeMap;

use betterlite_auth::session_token_codec::builders::{CliSession, SessionTokenCodec};
use betterlite_auth::session_token_codec::record::SessionRecord;
use betterlite_config::loader::ConfigLoader;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{NonceSource, StoredCipher};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use secrecy::ExposeSecret as _;
use serde_json::{Map, Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = SessionCodecInput;

    fn run(input: &Self::Input) -> HelperResult {
        run_codec(input)
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionCodecInput {
    pub(crate) salt_key: Option<String>,
    pub(crate) master_key: Option<String>,
    #[serde(default)]
    pub(crate) max_ui_session_budget: Option<Decimal>,
    #[serde(default)]
    pub(crate) env: BTreeMap<String, String>,
    pub(crate) nonce_hex: String,
    pub(crate) now: DateTime<Utc>,
    pub(crate) steps: Vec<SessionStep>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum SessionStep {
    UiToken {
        user: LiteLLMUserTable,
    },
    CliToken {
        user: LiteLLMUserTable,
        #[serde(default)]
        team_id: Option<String>,
        #[serde(default)]
        team_alias: Option<String>,
        #[serde(default)]
        team_models: Option<Vec<String>>,
        #[serde(default)]
        team_model_aliases: Option<BTreeMap<String, String>>,
        #[serde(default)]
        max_budget: Option<Decimal>,
    },
    Decode {
        token: String,
    },
}

struct FixedNonces(Vec<u8>);

impl NonceSource for FixedNonces {
    fn fill(&self, nonce: &mut [u8]) -> Result<(), SecretError> {
        let bytes = self.0.get(..nonce.len()).ok_or(SecretError::Truncated)?;
        nonce.copy_from_slice(bytes);
        Ok(())
    }
}

type Codec = SessionTokenCodec<FixedNonces, TracingDiagnostics>;

fn value_error(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

fn run_codec(input: &SessionCodecInput) -> HelperResult {
    let nonce = hex::decode(&input.nonce_hex)
        .map_err(|_source| value_error("nonce_hex is not valid hex"))?;
    let mut lookup = input.env.clone();
    if let Some(salt) = &input.salt_key {
        lookup.insert("LITELLM_SALT_KEY".to_owned(), salt.clone());
    }
    let source = config_source(input);
    let config = ConfigLoader { lookup }
        .load(&source)
        .map_err(|error| value_error(&error.to_string()))?;
    let codec = SessionTokenCodec {
        cipher: StoredCipher {
            settings: config.general_settings.cipher,
            nonces: FixedNonces(nonce),
            diagnostics: TracingDiagnostics,
        },
        max_ui_session_budget: config.sections.session_token_codec.max_ui_session_budget,
        cli_expiration_hours: config.sections.session_token_codec.cli_expiration_hours,
    };
    let results = input
        .steps
        .iter()
        .map(|step| run_step(&codec, input.now, step))
        .collect();
    Ok(Value::Array(results))
}

fn config_source(input: &SessionCodecInput) -> String {
    let mut litellm = Map::new();
    let budget = input
        .max_ui_session_budget
        .map_or(Value::Null, |budget| Value::String(budget.to_string()));
    litellm.insert("max_ui_session_budget".to_owned(), budget);
    let mut general = Map::new();
    if let Some(master_key) = &input.master_key {
        general.insert("master_key".to_owned(), Value::String(master_key.clone()));
    }
    json!({"general_settings": general, "litellm_settings": litellm}).to_string()
}

fn run_step(codec: &Codec, now: DateTime<Utc>, step: &SessionStep) -> Value {
    match answer(codec, now, step) {
        Ok(value) => json!({"ok": true, "value": value}),
        Err(_error) => json!({"ok": false}),
    }
}

fn answer(
    codec: &Codec,
    now: DateTime<Utc>,
    step: &SessionStep,
) -> Result<Value, Box<HelperError>> {
    match step {
        SessionStep::UiToken { user } => codec
            .ui_session_token(user, now)
            .map(|token| Value::String(token.expose_secret().to_owned()))
            .map_err(|error| value_error(&error.to_string())),
        SessionStep::CliToken {
            user,
            team_id,
            team_alias,
            team_models,
            team_model_aliases,
            max_budget,
        } => {
            let selection = CliSession {
                team_id: team_id.clone(),
                team_alias: team_alias.clone(),
                team_models: team_models.clone(),
                team_model_aliases: team_model_aliases.clone(),
                max_budget: *max_budget,
            };
            codec
                .cli_session_token(user, &selection, now)
                .map(|token| Value::String(token.expose_secret().to_owned()))
                .map_err(|error| value_error(&error.to_string()))
        }
        SessionStep::Decode { token } => {
            let record = codec
                .session_from_token(token)
                .map_err(|error| value_error(&error.to_string()))?;
            record.as_ref().map_or(Ok(Value::Null), |auth| {
                serde_json::to_value(SessionRecord::from_auth(auth))
                    .map_err(|error| value_error(&error.to_string()))
            })
        }
    }
}
