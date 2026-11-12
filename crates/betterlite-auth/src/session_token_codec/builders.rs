use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::{NonceSource, StoredCipher};
use chrono::{DateTime, TimeDelta, Utc};
use rust_decimal::Decimal;
use secrecy::SecretString;
use thiserror::Error;

use crate::resolved_key::auth::{AuthError, AuthInput, UserAPIKeyAuth};
use crate::session_token_codec::payload::{SessionShape, seal_session};
use crate::session_token_codec::{DecodeError, decode_session_token};
use crate::user_roles::UserRole;

const UI_TOKEN: &str = "ui-token";
const UI_TEAM_ID: &str = "litellm-dashboard";
const UI_RPM_LIMIT: i64 = 100;
const UI_SESSION_MINUTES: i64 = 10;
const CLI_SESSION_KEY_PREFIX: &str = "cli-session";
const CLI_RANDOM_BYTES: usize = 16;
const EXPIRY_FORMAT: &str = "%Y-%m-%dT%H:%M:%S%.3f+00:00";

#[derive(Debug, Clone, Copy, strum::Display)]
pub enum SessionKind {
    #[strum(serialize = "experimental UI")]
    ExperimentalUi,
    #[strum(serialize = "CLI JWT")]
    Cli,
}

#[derive(Debug, Clone, Default)]
pub struct CliSession {
    pub team_id: Option<String>,
    pub team_alias: Option<String>,
    pub team_models: Option<Vec<String>>,
    pub team_model_aliases: Option<BTreeMap<String, String>>,
    pub max_budget: Option<Decimal>,
}

#[derive(Debug, Error)]
pub enum SessionTokenError {
    #[error("a user role is required for {session} sessions")]
    MissingRole { session: SessionKind },
    #[error("the user role is not a valid role")]
    InvalidRole {
        #[source]
        source: strum::ParseError,
    },
    #[error("no secret is available to encrypt the session token")]
    MissingSecret,
    #[error("the session token could not be encrypted")]
    Encrypt {
        #[source]
        source: SecretError,
    },
    #[error("the session token could not be serialized")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the session token payload is invalid")]
    Payload {
        #[source]
        source: DecodeError,
    },
    #[error("the decrypted session token is not a valid session record")]
    Record {
        #[source]
        source: AuthError,
    },
    #[error("the session expiry is outside the supported range")]
    ExpiryRange,
}

pub struct SessionTokenCodec<N, D> {
    pub cipher: StoredCipher<N, D>,
    pub max_ui_session_budget: Option<Decimal>,
    pub cli_expiration_hours: i64,
}

impl<N, D> SessionTokenCodec<N, D> {
    pub fn session_from_token(
        &self,
        token: &str,
    ) -> Result<Option<UserAPIKeyAuth>, SessionTokenError> {
        let record = match decode_session_token(&self.cipher, token.as_bytes()) {
            Ok(record) => record,
            Err(DecodeError::Utf8 { .. } | DecodeError::NotAToken) => return Ok(None),
            Err(source) => return Err(SessionTokenError::Payload { source }),
        };
        UserAPIKeyAuth::construct(AuthInput::Dict(record))
            .map(Some)
            .map_err(|source| SessionTokenError::Record { source })
    }
}

struct CliScope {
    team_id: Option<String>,
    models: Vec<JsonData>,
    team_models: Vec<String>,
    team_model_aliases: Option<BTreeMap<String, JsonData>>,
}

impl<N, D> SessionTokenCodec<N, D>
where
    N: NonceSource,
{
    pub fn ui_session_token(
        &self,
        user: &LiteLLMUserTable,
        now: DateTime<Utc>,
    ) -> Result<SecretString, SessionTokenError> {
        let user_role = session_role(user.user_role.as_deref(), SessionKind::ExperimentalUi)?;
        let expires = expiry(now, TimeDelta::minutes(UI_SESSION_MINUTES))?;
        seal_session(
            &self.cipher,
            SessionShape {
                token: UI_TOKEN.to_owned(),
                key_name: UI_TOKEN.to_owned(),
                key_alias: UI_TOKEN.to_owned(),
                max_budget: self.max_ui_session_budget,
                expires,
                models: user.models.clone(),
                user_id: user.user_id.clone(),
                team_id: Some(UI_TEAM_ID.to_owned()),
                rpm_limit: Some(UI_RPM_LIMIT),
                team_alias: None,
                team_models: Vec::new(),
                team_model_aliases: None,
                user_role,
                is_session_token: false,
            },
        )
    }

    pub fn cli_session_token(
        &self,
        user: &LiteLLMUserTable,
        session: &CliSession,
        now: DateTime<Utc>,
    ) -> Result<SecretString, SessionTokenError> {
        let user_role = session_role(user.user_role.as_deref(), SessionKind::Cli)?;
        let expires = self.cli_expiry(now)?;
        let token = self.cli_token_id()?;
        let alias = format!("{CLI_SESSION_KEY_PREFIX}-{}", user.user_id);
        let scope = cli_scope(user, session);
        seal_session(
            &self.cipher,
            SessionShape {
                token,
                key_name: alias.clone(),
                key_alias: alias,
                max_budget: session.max_budget,
                expires,
                models: scope.models,
                user_id: user.user_id.clone(),
                team_id: scope.team_id,
                rpm_limit: None,
                team_alias: session.team_alias.clone(),
                team_models: scope.team_models,
                team_model_aliases: scope.team_model_aliases,
                user_role,
                is_session_token: true,
            },
        )
    }

    fn cli_expiry(&self, now: DateTime<Utc>) -> Result<String, SessionTokenError> {
        let hours = TimeDelta::try_hours(self.cli_expiration_hours)
            .ok_or(SessionTokenError::ExpiryRange)?;
        expiry(now, hours)
    }

    fn cli_token_id(&self) -> Result<String, SessionTokenError> {
        let mut random = [0_u8; CLI_RANDOM_BYTES];
        self.cipher
            .nonces
            .fill(&mut random)
            .map_err(|source| SessionTokenError::Encrypt { source })?;
        Ok(format!(
            "{CLI_SESSION_KEY_PREFIX}-{}",
            URL_SAFE_NO_PAD.encode(random)
        ))
    }
}

// an explicit team, even "", clears the user's models; only a missing team keeps them
fn cli_scope(user: &LiteLLMUserTable, session: &CliSession) -> CliScope {
    let team_id = session
        .team_id
        .clone()
        .or_else(|| user.teams.first().cloned());
    let models = if team_id.is_some() {
        Vec::new()
    } else {
        user.models.clone()
    };
    let team_models = session.team_models.clone().unwrap_or_default();
    let team_model_aliases: Option<BTreeMap<String, JsonData>> =
        session.team_model_aliases.as_ref().map(|aliases| {
            aliases
                .iter()
                .map(|(model, model_alias)| (model.clone(), JsonData::String(model_alias.clone())))
                .collect()
        });
    CliScope {
        team_id,
        models,
        team_models,
        team_model_aliases,
    }
}

fn session_role(role: Option<&str>, session: SessionKind) -> Result<UserRole, SessionTokenError> {
    let Some(text) = role else {
        return Err(SessionTokenError::MissingRole { session });
    };
    text.parse::<UserRole>()
        .map_err(|source| SessionTokenError::InvalidRole { source })
}

fn expiry(now: DateTime<Utc>, delta: TimeDelta) -> Result<String, SessionTokenError> {
    let expires = now
        .checked_add_signed(delta)
        .ok_or(SessionTokenError::ExpiryRange)?;
    Ok(expires.format(EXPIRY_FORMAT).to_string())
}
