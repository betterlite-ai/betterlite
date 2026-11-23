use std::sync::Mutex;

use betterlite_config::sections::budget_reset::BudgetResetSettings;
use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_db::key_repository::decode::decode_mapping;
use betterlite_logging::callback_config::FeatureFlags;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};
use chrono::{DateTime, NaiveDateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use secrecy::SecretString;

use crate::key_access::error::DetailShape;
use crate::key_fields::base::KeyRequestBase;
use crate::key_fields::decode::{TypedValue, decode_key_request};
use crate::key_fields::update::UpdateKeyRequest;
use crate::key_policy_fields::KeyClock;
use crate::management_guards::metadata::{METADATA_FIELDS, PREMIUM_METADATA_FIELDS};
use crate::router_weight_references::error::RouterWeightError;

use super::super::Row;
use super::super::error::KeyPatchError;
use super::super::fixtures::database::TestDatabase;
use super::super::metadata::{
    MetadataContext, PatchWarning, prepare_metadata_fields, python_isoformat,
};
use super::super::prepare::{PrepareContext, prepare_key_update_data};
use super::super::soft_budget::SoftBudgetStore;
use super::super::transaction::KeyPatchOutcome;

pub(super) type Cipher = StoredCipher<OsNonces, TracingDiagnostics>;

const FLAGS: FeatureFlags = FeatureFlags {
    otel_v2_enabled: false,
};

pub(super) struct Fixture {
    now: DateTime<Utc>,
    reset: BudgetResetSettings,
    database: TimeZoneDatabase,
}

impl Fixture {
    pub(super) const fn clock(&self) -> KeyClock<'_> {
        KeyClock {
            now: self.now,
            reset: &self.reset,
            database: &self.database,
        }
    }
}

pub(super) fn fixture() -> Fixture {
    Fixture {
        now: instant("2026-05-01T10:00:00.123456Z"),
        reset: BudgetResetSettings::default(),
        database: TimeZoneDatabase::from_env(),
    }
}

pub(super) fn instant(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .expect("a timestamp")
        .with_timezone(&Utc)
}

fn aware(text: &str) -> Timestamp {
    Timestamp::Aware(DateTime::parse_from_rfc3339(text).expect("an aware timestamp"))
}

fn naive(text: &str) -> Timestamp {
    Timestamp::Naive(
        NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f").expect("a naive timestamp"),
    )
}

pub(super) fn cipher() -> Cipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from("b894-signing-key")),
            algorithm: Algorithm::Aes256Gcm,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

fn failing_cipher() -> Cipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key: None,
            algorithm: Algorithm::Aes256Gcm,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

pub(super) fn json(document: &str) -> JsonData {
    serde_json::from_str(document).expect("a JSON document")
}

pub(super) fn json_text(value: &str) -> JsonData {
    JsonData::String(value.to_owned())
}

pub(super) fn object(document: &str) -> Row {
    let JsonData::Object(fields) = json(document) else {
        panic!("the fixture is a JSON object");
    };
    fields
}

fn metadata(document: &str) -> JsonData {
    JsonData::Object(object(document))
}

fn value(document: &str) -> TypedValue {
    decode_key_request(document.as_bytes()).expect("a JSON request")
}

pub(super) fn update_request(document: &str) -> UpdateKeyRequest {
    UpdateKeyRequest::read(&value(document)).expect("an update request")
}

pub(super) fn base_request(document: &str) -> KeyRequestBase {
    KeyRequestBase::read(&value(document)).expect("a key request base")
}

pub(super) fn existing(document: &str) -> LiteLLMVerificationToken {
    decode_mapping(object(document)).expect("a stored key row")
}

pub(super) fn assert_metadata(values: &Row, document: &str) {
    assert_eq!(values.get("metadata"), Some(&metadata(document)));
}

pub(super) fn string_field<'row>(values: &'row Row, name: &str) -> &'row str {
    match values.get(name) {
        Some(JsonData::String(text)) => text.as_str(),
        other => panic!("{name} is a string, found {other:?}"),
    }
}

pub(super) async fn prepared(
    fixture: &Fixture,
    stored: &LiteLLMVerificationToken,
    request: &UpdateKeyRequest,
) -> (Result<Row, KeyPatchError>, Vec<PatchWarning>) {
    let cipher = cipher();
    let mut warnings = Vec::new();
    let result = {
        let mut context = PrepareContext {
            existing: stored,
            clock: fixture.clock(),
            flags: FLAGS,
            models: None,
            configured: None,
            warnings: &mut warnings,
        };
        prepare_key_update_data(request, &cipher, &mut context).await
    };
    (result, warnings)
}

pub(super) async fn prepared_row(
    fixture: &Fixture,
    stored: &LiteLLMVerificationToken,
    request: &UpdateKeyRequest,
) -> Row {
    prepared(fixture, stored, request)
        .await
        .0
        .expect("a prepared patch")
}

pub(super) async fn prepare_error(
    fixture: &Fixture,
    stored: &LiteLLMVerificationToken,
    request: &UpdateKeyRequest,
) -> KeyPatchError {
    prepared(fixture, stored, request)
        .await
        .0
        .expect_err("a rejected patch")
}

fn run_metadata(
    request: &Row,
    existing_metadata: &Row,
    cipher: &Cipher,
) -> (Result<JsonData, KeyPatchError>, Vec<PatchWarning>) {
    let mut values = Row::new();
    let mut warnings = Vec::new();
    let prepared = {
        let mut context = MetadataContext {
            flags: FLAGS,
            cipher,
            warnings: &mut warnings,
        };
        prepare_metadata_fields(request, &mut values, existing_metadata, &mut context)
    };
    let projected = prepared.map(|()| values.remove("metadata").unwrap_or(JsonData::Null));
    (projected, warnings)
}

fn callback_secret(request: &Row, cipher: &Cipher) -> JsonData {
    let (result, _) = run_metadata(request, &Row::new(), cipher);
    let JsonData::Object(projected) = result.expect("prepared metadata") else {
        panic!("metadata is an object");
    };
    let Some(JsonData::Array(entries)) = projected.get("logging") else {
        panic!("logging is a list");
    };
    let Some(JsonData::Object(entry)) = entries.first() else {
        panic!("a logging entry");
    };
    let Some(JsonData::Object(vars)) = entry.get("callback_vars") else {
        panic!("callback vars");
    };
    vars.get("langfuse_secret")
        .cloned()
        .expect("the secret var")
}

pub(super) fn assert_all_catalog_fields() {
    let mut request = Row::new();
    for name in METADATA_FIELDS.iter().chain(PREMIUM_METADATA_FIELDS.iter()) {
        request.insert((*name).to_owned(), json_text(&format!("v-{name}")));
    }
    let cipher = cipher();
    let (result, warnings) = run_metadata(&request, &object(r#"{"team": "t1"}"#), &cipher);
    let JsonData::Object(projected) = result.expect("projected metadata") else {
        panic!("metadata is an object");
    };
    for name in METADATA_FIELDS.iter().chain(PREMIUM_METADATA_FIELDS.iter()) {
        assert_eq!(projected.get(*name), Some(&json_text(&format!("v-{name}"))));
    }
    assert_eq!(projected.get("team"), Some(&json_text("t1")));
    assert!(warnings.is_empty());
}

pub(super) fn assert_isoformat() {
    let cases = [
        (
            aware("2026-05-02T10:00:00.500000+02:00"),
            "2026-05-02T10:00:00.500000+02:00",
        ),
        (
            aware("2026-05-02T10:00:00+02:00"),
            "2026-05-02T10:00:00+02:00",
        ),
        (
            naive("2026-05-02T10:00:00.500000"),
            "2026-05-02T10:00:00.500000",
        ),
        (naive("2026-05-02T10:00:00"), "2026-05-02T10:00:00"),
    ];
    for (timestamp, expected) in cases {
        assert_eq!(python_isoformat(&timestamp), expected);
    }
}

pub(super) async fn assert_router_unavailable(
    fixture: &Fixture,
    stored: &LiteLLMVerificationToken,
    request: &UpdateKeyRequest,
) {
    let error = prepare_error(fixture, stored, request).await;
    assert!(matches!(
        error,
        KeyPatchError::RouterWeights(RouterWeightError::DatabaseUnavailable)
    ));
    assert_eq!(error.response(), (Some(503_u16), Some(DetailShape::Text)));
}

pub(super) async fn assert_router_invalid(
    fixture: &Fixture,
    stored: &LiteLLMVerificationToken,
    request: &UpdateKeyRequest,
) {
    let error = prepare_error(fixture, stored, request).await;
    assert!(matches!(
        error,
        KeyPatchError::RouterWeights(RouterWeightError::InvalidSettings(_))
    ));
    assert_eq!(error.response(), (Some(400_u16), Some(DetailShape::Text)));
}

pub(super) fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-9_f64,
        "{actual} != {expected}"
    );
}

pub(super) fn number_f64(row: &Row, name: &str) -> f64 {
    match row.get(name) {
        Some(JsonData::Number(number)) => number.as_f64().expect("a finite number"),
        other => panic!("{name} is a number, found {other:?}"),
    }
}

// the budget row is created first in the transaction, then the key row names it
pub(super) async fn assert_created_budget(database: &TestDatabase, outcome: &KeyPatchOutcome) {
    assert_eq!(outcome.data.get("key_alias"), Some(&json_text("linked")));
    let Some(JsonData::String(budget_id)) = outcome.data.get("budget_id") else {
        panic!("the key names its created budget");
    };
    let budget = database
        .budget_row(budget_id)
        .await
        .expect("the created budget");
    assert_close(number_f64(&budget, "soft_budget"), 3.0_f64);
    assert_eq!(budget.get("created_by"), Some(&json_text("admin")));
    assert_eq!(budget.get("updated_by"), Some(&json_text("admin")));
    let key = database.key_row("token-2").await.expect("the key row");
    assert_eq!(key.get("budget_id"), Some(&json_text(budget_id)));
}

// the pinned columns: the budget's soft_budget and updated_by, and the key's own patch with its stamps
pub(super) async fn assert_pinned_rows(database: &TestDatabase, outcome: &KeyPatchOutcome) {
    assert_eq!(outcome.token, "token-1");
    assert_eq!(outcome.data.get("key_alias"), Some(&json_text("renamed")));
    assert_eq!(outcome.data.get("budget_id"), Some(&json_text("budget-1")));
    let budget = database
        .budget_row("budget-1")
        .await
        .expect("the budget row");
    assert_close(number_f64(&budget, "soft_budget"), 0.7_f64);
    assert_eq!(budget.get("updated_by"), Some(&json_text("admin")));
    let key = database.key_row("token-1").await.expect("the key row");
    assert_eq!(key.get("key_alias"), Some(&json_text("renamed")));
    assert_eq!(key.get("budget_id"), Some(&json_text("budget-1")));
    let Some(JsonData::String(stored_metadata)) = key.get("metadata") else {
        panic!("the stored metadata string");
    };
    assert_eq!(
        json(stored_metadata),
        metadata(r#"{"team": "t1", "owner": "ops"}"#)
    );
    assert!(matches!(
        key.get("settings_updated_at"),
        Some(JsonData::String(_))
    ));
}

#[derive(Default)]
pub(super) struct FakeSoftBudgetStore {
    updates: Mutex<Vec<(String, Option<Decimal>, String)>>,
    created: Mutex<Vec<(Decimal, String)>>,
    next_id: String,
}

impl FakeSoftBudgetStore {
    pub(super) fn with_id(next_id: &str) -> Self {
        Self {
            next_id: next_id.to_owned(),
            ..Self::default()
        }
    }

    pub(super) fn updates(&self) -> Vec<(String, Option<Decimal>, String)> {
        self.updates.lock().unwrap().clone()
    }

    pub(super) fn created(&self) -> Vec<(Decimal, String)> {
        self.created.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl SoftBudgetStore for FakeSoftBudgetStore {
    async fn update_soft_budget(
        &self,
        budget_id: &str,
        soft_budget: Option<Decimal>,
        updated_by: &str,
    ) -> Result<(), KeyPatchError> {
        self.updates.lock().unwrap().push((
            budget_id.to_owned(),
            soft_budget,
            updated_by.to_owned(),
        ));
        Ok(())
    }

    async fn create_soft_budget(
        &self,
        soft_budget: Decimal,
        created_by: &str,
    ) -> Result<String, KeyPatchError> {
        self.created
            .lock()
            .unwrap()
            .push((soft_budget, created_by.to_owned()));
        Ok(self.next_id.clone())
    }
}
