#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::session_token_codec::builders::SessionTokenCodec;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, NonceSource, StoredCipher};
use chrono::{DateTime, Utc};
use secrecy::SecretString;

use crate::cli_team_grants::records::CliSsoTeamDetail;
use crate::mcp::outbound_credentials::WallClock;

use super::{
    ActiveSubjectSettings, ActiveSubjects, IdentityLookup, SubjectReadError, SubjectStore,
    UserRowSource,
};

pub(crate) const SALT: &str = "active-subjects-salt-key";
pub(crate) const ADMIN_NAME: &str = "default_user_id";

pub(crate) fn fixed_now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_780_000_000, 0).unwrap()
}

pub(crate) fn key_row(fields: &str) -> UserAPIKeyAuth {
    let values: BTreeMap<String, JsonData> = serde_json::from_str(fields).unwrap();
    UserAPIKeyAuth::construct(AuthInput::Dict(values)).unwrap()
}

pub(crate) fn user_row(fields: &str) -> LiteLLMUserTable {
    serde_json::from_str(fields).unwrap()
}

pub(crate) struct FixedClock(pub(crate) DateTime<Utc>);

impl WallClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

pub(crate) struct CountingNonces;

impl NonceSource for CountingNonces {
    fn fill(&self, nonce: &mut [u8]) -> Result<(), SecretError> {
        nonce
            .iter_mut()
            .zip(0_u8..)
            .for_each(|(byte, index)| *byte = index);
        Ok(())
    }
}

pub(crate) fn session_codec() -> SessionTokenCodec<CountingNonces, TracingDiagnostics> {
    SessionTokenCodec {
        cipher: StoredCipher {
            settings: CipherSettings {
                signing_key: Some(SecretString::from(SALT)),
                algorithm: Algorithm::Aes256Gcm,
            },
            nonces: CountingNonces,
            diagnostics: TracingDiagnostics,
        },
        max_ui_session_budget: None,
        cli_expiration_hours: 24,
    }
}

#[derive(Clone)]
pub(crate) enum KeyAnswer {
    Found(UserAPIKeyAuth),
    Failed(SubjectReadError),
}

#[derive(Clone)]
pub(crate) enum UserAnswer {
    Found(LiteLLMUserTable),
    Missing,
    Failed(SubjectReadError),
}

#[derive(Default)]
pub(crate) struct Calls {
    pub(crate) keys: Vec<String>,
    pub(crate) users: Vec<(String, UserRowSource)>,
    pub(crate) identities: Vec<String>,
    pub(crate) teams: Vec<Vec<String>>,
}

pub(crate) struct FakeStore {
    pub(crate) has_database: bool,
    pub(crate) key: KeyAnswer,
    pub(crate) user: UserAnswer,
    pub(crate) identity: IdentityLookup,
    pub(crate) details: Option<Vec<CliSsoTeamDetail>>,
    pub(crate) calls: Arc<Mutex<Calls>>,
}

impl FakeStore {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            has_database: true,
            key: KeyAnswer::Failed(SubjectReadError::Other),
            user: UserAnswer::Missing,
            identity: IdentityLookup::NotFound,
            details: Some(Vec::new()),
            calls: Arc::new(Mutex::new(Calls::default())),
        }
    }

    pub(crate) fn calls(&self) -> Arc<Mutex<Calls>> {
        Arc::clone(&self.calls)
    }
}

impl Default for FakeStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SubjectStore for FakeStore {
    fn now(&self) -> DateTime<Utc> {
        fixed_now()
    }

    fn has_database(&self) -> bool {
        self.has_database
    }

    fn key_by_hash(
        &self,
        key_hash: &str,
    ) -> impl Future<Output = Result<UserAPIKeyAuth, SubjectReadError>> + Send {
        self.calls.lock().unwrap().keys.push(key_hash.to_owned());
        let answer = match &self.key {
            KeyAnswer::Found(key) => Ok(key.clone()),
            KeyAnswer::Failed(error) => Err(*error),
        };
        ready(answer)
    }

    fn user_by_id(
        &self,
        user_id: &str,
        source: UserRowSource,
    ) -> impl Future<Output = Result<Option<LiteLLMUserTable>, SubjectReadError>> + Send {
        self.calls
            .lock()
            .unwrap()
            .users
            .push((user_id.to_owned(), source));
        let answer = match &self.user {
            UserAnswer::Found(row) => Ok(Some(row.clone())),
            UserAnswer::Missing => Ok(None),
            UserAnswer::Failed(error) => Err(*error),
        };
        ready(answer)
    }

    fn identity_by_hash(&self, key_hash: &str) -> impl Future<Output = IdentityLookup> + Send {
        self.calls
            .lock()
            .unwrap()
            .identities
            .push(key_hash.to_owned());
        ready(self.identity)
    }

    fn team_details(
        &self,
        teams: &[String],
    ) -> impl Future<Output = Option<Vec<CliSsoTeamDetail>>> + Send {
        self.calls.lock().unwrap().teams.push(teams.to_vec());
        ready(self.details.clone())
    }
}

pub(crate) fn subjects<S: SubjectStore>(store: S) -> ActiveSubjects<S> {
    ActiveSubjects {
        store,
        settings: ActiveSubjectSettings {
            admin_name: ADMIN_NAME.to_owned(),
            ..ActiveSubjectSettings::default()
        },
        secret: Some(SecretString::from(SALT)),
        clock: Arc::new(FixedClock(fixed_now())),
        nonces: Arc::new(CountingNonces),
    }
}

pub(crate) fn detail(team_id: &str, alias: &str, models: &[&str]) -> CliSsoTeamDetail {
    CliSsoTeamDetail {
        team_id: Some(team_id.to_owned()),
        team_alias: Some(alias.to_owned()),
        team_models: models.iter().map(|model| (*model).to_owned()).collect(),
        team_model_aliases: None,
    }
}
