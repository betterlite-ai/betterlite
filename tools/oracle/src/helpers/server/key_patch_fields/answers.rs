use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_config::sections::budget_reset::BudgetResetSettings;
use betterlite_db::key_repository::decode::decode_mapping;
use betterlite_logging::callback_config::FeatureFlags;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};
use betterlite_server::key_fields::decode::TypedValue;
use betterlite_server::key_fields::update::UpdateKeyRequest;
use betterlite_server::key_patch_fields::{
    Row,
    accessors::resolve_token_to_update,
    error::KeyPatchError,
    metadata::{MetadataContext, PatchWarning, prepare_metadata_fields},
    prepare::{PrepareContext, metadata_fields, prepare_key_update_data},
    soft_budget::{SoftBudgetStore, apply_soft_budget_update},
};
use betterlite_server::key_policy_fields::KeyClock;
use betterlite_server::management_guards::amounts::Amount;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde::Serialize;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use super::{Call, CallFailed};

const FLAGS: FeatureFlags = FeatureFlags {
    otel_v2_enabled: false,
};

type Cipher = StoredCipher<OsNonces, TracingDiagnostics>;

fn cipher() -> Cipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from("b894-oracle-signing-key")),
            algorithm: Algorithm::Aes256Gcm,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

fn error_json(error: &KeyPatchError) -> Value {
    let (status, _) = error.response();
    let exception = error.proxy_exception();
    let (kind, param, code) = exception.map_or((None, None, None), |exception| {
        (
            Some(exception.error_type),
            exception.param,
            Some(exception.code),
        )
    });
    json!({"status": status, "type": kind, "param": param, "code": code})
}

fn typed_body(body: &Value) -> Result<TypedValue, CallFailed> {
    serde_json::from_value(body.clone())
        .map(|value| TypedValue { json: value })
        .map_err(CallFailed::from)
}

pub(super) fn answer(call: &Call) -> Result<Value, CallFailed> {
    match call {
        Call::Metadata {
            request,
            existing_metadata,
        } => metadata_answer(request, existing_metadata),
        Call::SoftBudgetUpdate {
            request,
            existing,
            non_default_values,
        } => soft_budget_answer(request, existing, non_default_values),
        Call::Token {
            request_key,
            existing_token,
        } => Ok(token_answer(
            request_key.as_deref(),
            existing_token.as_deref(),
        )),
        Call::Prepare { request, existing } => prepare_answer(request, existing),
    }
}

fn answered<T: Serialize>(
    outcome: Result<T, KeyPatchError>,
    warnings: &[PatchWarning],
    effects: Vec<Value>,
) -> Result<Value, CallFailed> {
    let (value, error) = match outcome {
        Ok(value) => (serde_json::to_value(value)?, None),
        Err(error) => (Value::Null, Some(error_json(&error))),
    };
    let subjects: Vec<Value> = warnings
        .iter()
        .map(|warning| Value::String((*warning).to_owned()))
        .collect();
    Ok(json!({"value": value, "error": error, "warnings": subjects, "effects": effects}))
}

fn runtime() -> Result<Runtime, CallFailed> {
    Builder::new_current_thread()
        .enable_time()
        .build()
        .map_err(CallFailed::from)
}

fn metadata_answer(body: &Value, existing_metadata: &Row) -> Result<Value, CallFailed> {
    let request = UpdateKeyRequest::read(&typed_body(body)?)?;
    let mut values = request.project()?.supplied;
    let fields = metadata_fields(&request, values.clone());
    let cipher = cipher();
    let mut warnings = Vec::new();
    let outcome = prepare_metadata_fields(
        &fields,
        &mut values,
        existing_metadata,
        &mut MetadataContext {
            flags: FLAGS,
            cipher: &cipher,
            warnings: &mut warnings,
        },
    );
    answered(outcome.map(|()| values), &warnings, Vec::new())
}

fn soft_budget_answer(
    body: &Value,
    stored: &Row,
    non_default_values: &Row,
) -> Result<Value, CallFailed> {
    let request = UpdateKeyRequest::read(&typed_body(body)?)?;
    let existing = decode_mapping(stored.clone())?;
    let soft = match request.update.soft_budget {
        Some(Some(amount)) => Some(Amount::Finite(amount.0)),
        Some(None) | None => None,
    };
    let store = RecordingStore::default();
    let outcome = runtime()?.block_on(apply_soft_budget_update(
        &store,
        soft,
        non_default_values.clone(),
        existing.budget_id.as_deref(),
        "admin",
    ));
    answered(outcome, &[], store.snapshot())
}

fn token_answer(request_key: Option<&str>, existing_token: Option<&str>) -> Value {
    match resolve_token_to_update(request_key, existing_token) {
        Ok(token) => json!({"value": token, "error": null, "warnings": [], "effects": []}),
        Err(error) => {
            json!({"value": null, "error": error_json(&error), "warnings": [], "effects": []})
        }
    }
}

fn prepare_answer(body: &Value, stored: &Row) -> Result<Value, CallFailed> {
    let request = UpdateKeyRequest::read(&typed_body(body)?)?;
    let existing = decode_mapping(stored.clone())?;
    let reset = BudgetResetSettings::default();
    let database = TimeZoneDatabase::from_env();
    let now = DateTime::parse_from_rfc3339("2026-05-01T10:00:00.123456Z")?.with_timezone(&Utc);
    let clock = KeyClock {
        now,
        reset: &reset,
        database: &database,
    };
    let cipher = cipher();
    let mut warnings = Vec::new();
    let mut context = PrepareContext {
        existing: &existing,
        clock,
        flags: FLAGS,
        models: None,
        configured: None,
        warnings: &mut warnings,
    };
    let outcome = runtime()?.block_on(prepare_key_update_data(&request, &cipher, &mut context));
    answered(outcome, &warnings, Vec::new())
}

#[derive(Default)]
struct RecordingStore {
    effects: Mutex<Vec<Value>>,
}

impl RecordingStore {
    fn record(&self, effect: Value) {
        self.effects
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(effect);
    }

    fn snapshot(&self) -> Vec<Value> {
        self.effects
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl SoftBudgetStore for RecordingStore {
    async fn update_soft_budget(
        &self,
        budget_id: &str,
        soft_budget: Option<Decimal>,
        updated_by: &str,
    ) -> Result<(), KeyPatchError> {
        let effect = json!({"kind": "update", "budget_id": budget_id, "soft_budget": soft_budget.and_then(|value| value.to_f64()), "actor": updated_by});
        self.record(effect);
        Ok(())
    }

    async fn create_soft_budget(
        &self,
        soft_budget: Decimal,
        created_by: &str,
    ) -> Result<String, KeyPatchError> {
        let effect = json!({"kind": "create", "budget_id": "budget-created", "soft_budget": soft_budget.to_f64(), "actor": created_by});
        self.record(effect);
        Ok(String::from("budget-created"))
    }
}
