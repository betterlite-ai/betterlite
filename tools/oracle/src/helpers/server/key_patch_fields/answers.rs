use betterlite_config::sections::budget_reset::BudgetResetSettings;
use betterlite_db::key_repository::decode::decode_mapping;
use betterlite_logging::callback_config::FeatureFlags;
use betterlite_server::key_fields::base::KeyRequestBase;
use betterlite_server::key_fields::update::UpdateKeyRequest;
use betterlite_server::key_patch_fields::Row;
use betterlite_server::key_patch_fields::accessors::{
    is_different_team, requested_end_user_budget_id, require_prisma_client, resolve_token_to_update,
};
use betterlite_server::key_patch_fields::metadata::{MetadataContext, prepare_metadata_fields};
use betterlite_server::key_patch_fields::prepare::metadata_request_fields;
use betterlite_server::key_patch_fields::prepare::{PrepareContext, prepare_key_update_data};
use betterlite_server::key_patch_fields::soft_budget::{
    apply_soft_budget_update, validate_max_budget, validate_soft_budget_value,
};
use betterlite_server::key_policy_fields::KeyClock;
use betterlite_server::management_guards::amounts::Amount;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use super::store::RecordingStore;
use super::support::{cipher, envelope, error_json, typed_body, warnings_json};
use super::{BudgetField, Call, CallFailed};

const FLAGS: FeatureFlags = FeatureFlags {
    otel_v2_enabled: false,
};

pub(super) fn answer(call: &Call) -> Result<Value, CallFailed> {
    match call {
        Call::Metadata {
            request,
            existing_metadata,
        } => metadata_answer(request, existing_metadata),
        Call::BudgetCheck { field, value } => Ok(budget_answer(*field, *value)),
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
        Call::Team {
            request_team,
            existing_team,
        } => Ok(team_answer(
            request_team.as_deref(),
            existing_team.as_deref(),
        )),
        Call::EndUser { request } => end_user_answer(request),
        Call::Database => Ok(database_answer()),
        Call::Prepare { request, existing } => prepare_answer(request, existing),
    }
}

fn metadata_answer(body: &Value, existing_metadata: &Row) -> Result<Value, CallFailed> {
    let typed = typed_body(body)?;
    let request = UpdateKeyRequest::read(&typed)?;
    let mut values = request.project()?.supplied;
    let fields = metadata_request_fields(&request)?;
    let cipher = cipher();
    let mut warnings = Vec::new();
    let outcome = {
        let mut context = MetadataContext {
            flags: FLAGS,
            cipher: &cipher,
            warnings: &mut warnings,
        };
        prepare_metadata_fields(&fields, &mut values, existing_metadata, &mut context)
    };
    match outcome {
        Ok(()) => Ok(envelope(
            serde_json::to_value(values)?,
            None,
            warnings_json(&warnings),
            Vec::new(),
        )),
        Err(error) => Ok(envelope(
            Value::Null,
            Some(error_json(&error)),
            warnings_json(&warnings),
            Vec::new(),
        )),
    }
}

fn budget_answer(field: BudgetField, value: Option<Decimal>) -> Value {
    let amount = value.map(Amount::Finite);
    let outcome = match field {
        BudgetField::Soft => validate_soft_budget_value(amount),
        BudgetField::Max => validate_max_budget(amount),
    };
    match outcome {
        Ok(()) => envelope(Value::Null, None, Vec::new(), Vec::new()),
        Err(error) => envelope(
            Value::Null,
            Some(error_json(&error)),
            Vec::new(),
            Vec::new(),
        ),
    }
}

fn soft_budget_answer(
    body: &Value,
    stored: &Row,
    non_default_values: &Row,
) -> Result<Value, CallFailed> {
    let typed = typed_body(body)?;
    let request = UpdateKeyRequest::read(&typed)?;
    let existing = decode_mapping(stored.clone())?;
    let soft_budget = request
        .update
        .soft_budget
        .flatten()
        .map(|amount| Amount::Finite(amount.0));
    let store = RecordingStore::default();
    let runtime = Builder::new_current_thread()
        .enable_time()
        .build()
        .map_err(CallFailed::from)?;
    let outcome = runtime.block_on(apply_soft_budget_update(
        &store,
        soft_budget,
        non_default_values.clone(),
        existing.budget_id.as_deref(),
        "admin",
    ));
    let effects = store.snapshot();
    match outcome {
        Ok(values) => Ok(envelope(
            serde_json::to_value(values)?,
            None,
            Vec::new(),
            effects,
        )),
        Err(error) => Ok(envelope(
            Value::Null,
            Some(error_json(&error)),
            Vec::new(),
            effects,
        )),
    }
}

fn token_answer(request_key: Option<&str>, existing_token: Option<&str>) -> Value {
    match resolve_token_to_update(request_key, existing_token) {
        Ok(token) => envelope(json!(token), None, Vec::new(), Vec::new()),
        Err(error) => envelope(
            Value::Null,
            Some(error_json(&error)),
            Vec::new(),
            Vec::new(),
        ),
    }
}

fn team_answer(request_team: Option<&str>, existing_team: Option<&str>) -> Value {
    envelope(
        json!(is_different_team(request_team, existing_team)),
        None,
        Vec::new(),
        Vec::new(),
    )
}

fn end_user_answer(body: &Value) -> Result<Value, CallFailed> {
    let typed = typed_body(body)?;
    let request = KeyRequestBase::read(&typed)?;
    Ok(envelope(
        json!(requested_end_user_budget_id(&request)),
        None,
        Vec::new(),
        Vec::new(),
    ))
}

fn database_answer() -> Value {
    match require_prisma_client(None) {
        Ok(_) => envelope(Value::Null, None, Vec::new(), Vec::new()),
        Err(error) => envelope(
            Value::Null,
            Some(error_json(&error)),
            Vec::new(),
            Vec::new(),
        ),
    }
}

fn prepare_answer(body: &Value, stored: &Row) -> Result<Value, CallFailed> {
    let typed = typed_body(body)?;
    let request = UpdateKeyRequest::read(&typed)?;
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
    let runtime = Builder::new_current_thread()
        .enable_time()
        .build()
        .map_err(CallFailed::from)?;
    let outcome = runtime.block_on(prepare_key_update_data(&request, &cipher, &mut context));
    match outcome {
        Ok(values) => Ok(envelope(
            serde_json::to_value(values)?,
            None,
            warnings_json(&warnings),
            Vec::new(),
        )),
        Err(error) => Ok(envelope(
            Value::Null,
            Some(error_json(&error)),
            warnings_json(&warnings),
            Vec::new(),
        )),
    }
}
