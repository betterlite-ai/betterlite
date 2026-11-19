use std::collections::BTreeMap;
use std::error::Error;

use betterlite_config::sections::budget_reset::BudgetResetSettings;
use betterlite_config::sections::key_policy_fields::{Settings, parse_key_alias_pattern};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_rows::member::BudgetLimitEntry;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::key_fields::decode::TypedValue;
use betterlite_server::key_fields::policy::{PolicyOperation, PolicyRequestBody};
use betterlite_server::key_fields::regenerate::RegenerateKeyRequest;
use betterlite_server::key_fields::request::GenerateKeyRequest;
use betterlite_server::key_fields::update::UpdateKeyRequest;
use betterlite_server::key_policy_fields::KeyClock;
use betterlite_server::key_policy_fields::alias::validate_key_alias_format;
use betterlite_server::key_policy_fields::columns::{
    decode_json_string_column, update_policy_request, verification_token_from_row,
};
use betterlite_server::key_policy_fields::generate::{generate_budget_windows, resolved_key_type};
use betterlite_server::key_policy_fields::key_type::handle_key_type;
use betterlite_server::key_policy_fields::model_budget::validate_model_max_budget;
use betterlite_server::key_policy_fields::regenerate::regenerate_as_update_request;
use betterlite_server::key_policy_fields::rotation::{RotationSwitches, set_key_rotation_fields};
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

type Row = BTreeMap<String, JsonData>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    Alias {
        pattern: Option<String>,
        enabled: bool,
        alias: Option<String>,
    },
    Rotation {
        data: Row,
        auto_rotate: bool,
        rotation_interval: Option<String>,
        existing_alias: Option<String>,
        #[serde(rename = "store_virtual_keys")]
        store: bool,
        now: String,
    },
    EffectivePolicy {
        operation: PolicyOperation,
        existing: Row,
        patch: Row,
        request: Value,
    },
    ModelBudget {
        budgets: Option<Row>,
    },
    JsonColumn {
        column: String,
        value: JsonData,
    },
    VerificationRow {
        row: Row,
    },
    KeyType {
        request: Value,
        columns: Row,
    },
    Regenerate {
        key: String,
        body: Value,
    },
    BudgetWindows {
        budget_limits: Option<Vec<BudgetLimitEntry>>,
        now: String,
    },
}

#[derive(Debug)]
struct CallFailed;

// the oracle reports one failed outcome per call, so every error collapses into this marker
impl<E: Error> From<E> for CallFailed {
    fn from(_: E) -> Self {
        Self
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let outcomes = input
            .calls
            .iter()
            .map(|call| match answer(call) {
                Ok(Some(value)) => json!({"ok": true, "value": value}),
                Ok(None) => json!({"ok": true}),
                Err(CallFailed) => json!({"ok": false}),
            })
            .collect();
        Ok(Value::Array(outcomes))
    }
}

fn answer(call: &Call) -> Result<Option<Value>, CallFailed> {
    match call {
        Call::Alias {
            pattern,
            enabled,
            alias,
        } => alias_answer(pattern.as_deref(), *enabled, alias.as_deref()),
        Call::Rotation {
            data,
            auto_rotate,
            rotation_interval,
            existing_alias,
            store,
            now,
        } => rotation_answer(
            data,
            RotationSwitches {
                auto_rotate: *auto_rotate,
                store_virtual_keys: *store,
            },
            rotation_interval.as_deref(),
            existing_alias.as_deref(),
            now,
        ),
        Call::EffectivePolicy {
            operation,
            existing,
            patch,
            request,
        } => policy_answer(*operation, existing, patch, request),
        Call::ModelBudget { budgets } => model_budget_answer(budgets.as_ref()),
        Call::JsonColumn { column, value } => json_column_answer(column, value),
        Call::VerificationRow { row } => value_answer(verification_token_from_row(row)?),
        Call::KeyType { request, columns } => key_type_answer(request, columns),
        Call::Regenerate { key, body } => regenerate_answer(key, body),
        Call::BudgetWindows { budget_limits, now } => budget_answer(budget_limits.as_deref(), now),
    }
}

fn typed_body(body: &Value) -> Result<TypedValue, CallFailed> {
    serde_json::from_value(body.clone())
        .map(|json| TypedValue { json })
        .map_err(CallFailed::from)
}

fn value_answer<T: Serialize>(value: T) -> Result<Option<Value>, CallFailed> {
    serde_json::to_value(value)
        .map(Some)
        .map_err(CallFailed::from)
}

fn alias_answer(
    pattern: Option<&str>,
    enabled: bool,
    alias: Option<&str>,
) -> Result<Option<Value>, CallFailed> {
    let compiled = match pattern {
        Some(source) => parse_key_alias_pattern(Value::String(source.to_owned()))?,
        None => None,
    };
    let settings = Settings {
        alias_format_validation: enabled,
        alias_pattern: compiled,
    };
    validate_key_alias_format(alias, settings.alias_rule().as_ref()).map_err(|error| *error)?;
    Ok(None)
}

fn rotation_answer(
    data: &Row,
    switches: RotationSwitches,
    rotation_interval: Option<&str>,
    existing_alias: Option<&str>,
    now: &str,
) -> Result<Option<Value>, CallFailed> {
    let mut fields = data.clone();
    let reset = BudgetResetSettings::default();
    let database = TimeZoneDatabase::from_env();
    let clock = clock_at(now, &reset, &database)?;
    set_key_rotation_fields(
        &mut fields,
        switches,
        rotation_interval,
        existing_alias,
        clock,
    )?;
    Ok(Some(serde_json::to_value(&fields)?))
}

fn policy_answer(
    operation: PolicyOperation,
    existing: &Row,
    patch: &Row,
    request: &Value,
) -> Result<Option<Value>, CallFailed> {
    let token = verification_token_from_row(existing)?;
    let typed = typed_body(request)?;
    let body = match operation {
        PolicyOperation::Update => PolicyRequestBody::Update(UpdateKeyRequest::read(&typed)?),
        PolicyOperation::Regenerate => {
            PolicyRequestBody::Regenerate(RegenerateKeyRequest::read(&typed)?)
        }
        PolicyOperation::Generate => return Err(CallFailed),
    };
    let resolved = match &body {
        PolicyRequestBody::Update(inner) => inner.project(),
        PolicyRequestBody::Regenerate(inner) => inner.project(),
        PolicyRequestBody::Generate(inner) => inner.project(),
    }?
    .resolved;
    let record = update_policy_request(operation, &token, patch, body)?;
    Ok(Some(json!({
        "operation": record.operation,
        "existing_key": record.existing_key,
        "effective_key": record.effective_key,
        "request": resolved,
    })))
}

fn model_budget_answer(budgets: Option<&Row>) -> Result<Option<Value>, CallFailed> {
    validate_model_max_budget(budgets)?;
    Ok(None)
}

fn json_column_answer(column: &str, value: &JsonData) -> Result<Option<Value>, CallFailed> {
    let decoded = decode_json_string_column(column, value)?;
    let serialized = serde_json::to_value(&decoded)?;
    Ok(Some(serialized))
}

fn key_type_answer(body: &Value, columns: &Row) -> Result<Option<Value>, CallFailed> {
    let request = GenerateKeyRequest::read(&typed_body(body)?)?;
    let handled = handle_key_type(resolved_key_type(&request), columns.clone());
    Ok(Some(serde_json::to_value(&handled)?))
}

fn regenerate_answer(key: &str, body: &Value) -> Result<Option<Value>, CallFailed> {
    let request = RegenerateKeyRequest::read(&typed_body(body)?)?;
    let patch = regenerate_as_update_request(key, &request)?;
    let Some(update) = patch else {
        return Ok(Some(Value::Null));
    };
    value_answer(update.project()?.resolved)
}

fn budget_answer(
    budget_limits: Option<&[BudgetLimitEntry]>,
    now: &str,
) -> Result<Option<Value>, CallFailed> {
    let reset = BudgetResetSettings::default();
    let database = TimeZoneDatabase::from_env();
    let clock = clock_at(now, &reset, &database)?;
    let windows = generate_budget_windows(budget_limits, clock)?;
    value_answer(windows)
}

// the oracle always runs the default UTC reset policy; the case freezes the request time
fn clock_at<'context>(
    now: &str,
    reset: &'context BudgetResetSettings,
    database: &'context TimeZoneDatabase,
) -> Result<KeyClock<'context>, CallFailed> {
    let parsed = DateTime::parse_from_rfc3339(now)?.with_timezone(&Utc);
    Ok(KeyClock {
        now: parsed,
        reset,
        database,
    })
}
