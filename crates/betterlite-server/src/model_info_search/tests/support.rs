use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use betterlite_auth::user_roles::UserRole;
use betterlite_db::resource_rows::model::ProxyModelTable;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces};
use secrecy::SecretString;
use serde_json::Number;
use tokio_util::task::TaskTracker;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt;
use tracing_subscriber::fmt::MakeWriter;

use crate::db_row_decrypt::warnings::Warnings;
use crate::model_team_scope::lookup::Decryption;
use crate::team_access::caller::TeamCaller;

pub(super) fn object(entries: Vec<(&str, Json)>) -> JsonObject {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

pub(super) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(super) fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

pub(super) fn decimal_text(value: &str) -> Json {
    Json::Number(value.parse::<Number>().expect("a JSON number"))
}

pub(super) fn flag(value: bool) -> Json {
    Json::Bool(value)
}

pub(super) fn list(items: Vec<Json>) -> Json {
    Json::List(items)
}

pub(super) fn params(model: &str) -> Json {
    Json::Object(object(vec![("model", text(model))]))
}

pub(super) fn model(name: &str, info: Vec<(&str, Json)>) -> JsonObject {
    object(vec![
        ("model_name", text(name)),
        ("litellm_params", params("openai/gpt-4o")),
        ("model_info", Json::Object(object(info))),
    ])
}

pub(super) fn named(name: &str) -> JsonObject {
    model(name, Vec::new())
}

pub(super) fn names(models: &[JsonObject]) -> Vec<String> {
    models
        .iter()
        .map(|model| match model.get("model_name") {
            Some(Json::Text(name)) => name.clone(),
            _ => String::new(),
        })
        .collect()
}

// A stored row as the DB returns it, with the given JSON text for params and model_info.
pub(super) fn stored_row(
    model_id: &str,
    model_name: &str,
    params: &str,
    info: &str,
) -> ProxyModelTable {
    let text = format!(
        r#"{{"model_id":"{model_id}","model_name":"{model_name}","litellm_params":{params},"model_info":{info}}}"#
    );
    serde_json::from_str(&text).expect("a stored model row")
}

struct NoOidcHttp;

impl OidcHttp for NoOidcHttp {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "a model info search test has no OIDC provider".to_owned(),
        })
    }
}

struct NoAzureTokens;

impl AzureOidcTokens for NoAzureTokens {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

struct FixedClock;

impl OidcClock for FixedClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

// ProxyConfig's decryption inputs: a plaintext fixture key, no secret manager, and an OIDC read that always fails.
pub(super) fn decryption() -> Decryption {
    Decryption {
        cipher: Arc::new(DbCipher {
            settings: CipherSettings {
                signing_key: Some(SecretString::from("b849-test-key".to_owned())),
                algorithm: Algorithm::XSalsa20Poly1305,
            },
            nonces: OsNonces,
            diagnostics: TracingDiagnostics,
        }),
        secrets: Arc::new(SecretLookup {
            manager: None,
            oidc_http: Arc::new(NoOidcHttp),
            azure_tokens: Arc::new(NoAzureTokens),
            clock: Arc::new(FixedClock),
            oidc_cache: OidcTokenCache::default(),
        }),
        environment: Arc::new(BTreeMap::new()),
        warnings: Arc::new(Warnings::default()),
        tasks: TaskTracker::new(),
    }
}

pub(super) fn caller(user_id: &str, team_id: &str) -> TeamCaller {
    TeamCaller {
        user_id: Some(user_id.to_owned()),
        user_role: Some(UserRole::InternalUser),
        team_id: Some(team_id.to_owned()),
        team_service_account: false,
    }
}

pub(super) fn db_model(name: &str, model_id: &str) -> JsonObject {
    model(
        name,
        vec![("db_model", Json::Bool(true)), ("id", text(model_id))],
    )
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(super) fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        str::from_utf8(&bytes)
            .expect("the captured log lines are UTF-8")
            .to_owned()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for Captured {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

pub(super) fn capture_logs() -> (Captured, DefaultGuard) {
    let buffer = Captured::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(Level::ERROR)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}
