#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::budget_reset::BudgetResetSettings;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use secrecy::SecretString;

use super::KeyClock;
use crate::key_fields::decode::{TypedValue, decode_key_request};
use crate::key_fields::regenerate::RegenerateKeyRequest;
use crate::key_fields::request::GenerateKeyRequest;
use crate::key_fields::update::UpdateKeyRequest;

mod alias;
mod budget;
mod columns;
mod generate;
mod hooks;
mod rotation;
mod unique_alias;

// the request time, the default reset policy and the calendar the key helpers read
struct Fixture {
    now: DateTime<Utc>,
    reset: BudgetResetSettings,
    database: TimeZoneDatabase,
}

impl Fixture {
    // a request time with microseconds that are not a whole millisecond, the finest digit Python keeps
    const fn clock(&self) -> KeyClock<'_> {
        KeyClock {
            now: self.now,
            reset: &self.reset,
            database: &self.database,
        }
    }
}

fn fixture() -> Fixture {
    Fixture {
        now: instant("2026-05-01T10:00:00.123456Z"),
        reset: BudgetResetSettings::default(),
        database: TimeZoneDatabase::from_env(),
    }
}

fn value(document: &str) -> TypedValue {
    decode_key_request(document.as_bytes()).expect("the fixture is a JSON object")
}

fn object(document: &str) -> BTreeMap<String, JsonData> {
    let JsonData::Object(fields) =
        serde_json::from_str::<JsonData>(document).expect("a JSON object")
    else {
        panic!("the fixture is an object");
    };
    fields
}

fn json(document: &str) -> JsonData {
    serde_json::from_str(document).expect("a JSON document")
}

fn generate_request(document: &str) -> GenerateKeyRequest {
    GenerateKeyRequest::read(&value(document)).expect("a generate request")
}

fn update_request(document: &str) -> UpdateKeyRequest {
    UpdateKeyRequest::read(&value(document)).expect("an update request")
}

fn regenerate_request(document: &str) -> RegenerateKeyRequest {
    RegenerateKeyRequest::read(&value(document)).expect("a regenerate request")
}

fn instant(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .expect("a timestamp")
        .with_timezone(&Utc)
}

fn cipher() -> StoredCipher<OsNonces, TracingDiagnostics> {
    StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from("b1224-signing-key")),
            algorithm: Algorithm::Aes256Gcm,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}
