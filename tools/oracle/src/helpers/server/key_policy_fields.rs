use std::collections::BTreeMap;
use std::sync::OnceLock;

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
use betterlite_server::key_policy_fields::rotation::set_key_rotation_fields;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use serde::Deserialize;
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
        store_virtual_keys: bool,
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

#[derive(Debug, thiserror::Error)]
#[error("the key policy helper call failed")]
struct CallFailed;

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
        } => {
            let compiled = match pattern.clone() {
                Some(source) => {
                    parse_key_alias_pattern(Value::String(source)).map_err(|_| CallFailed)?
                }
                None => None,
            };
            let settings = Settings {
                alias_format_validation: *enabled,
                alias_pattern: compiled,
            };
            validate_key_alias_format(alias.as_deref(), settings.alias_rule().as_ref())
                .map_err(|_| CallFailed)?;
            Ok(None)
        }
        Call::Rotation {
            data,
            auto_rotate,
            rotation_interval,
            existing_alias,
            store_virtual_keys,
            now,
        } => {
            let mut data = data.clone();
            let clock = clock_at(now)?;
            set_key_rotation_fields(
                &mut data,
                *auto_rotate,
                rotation_interval.as_deref(),
                existing_alias.as_deref(),
                *store_virtual_keys,
                clock,
            )
            .map_err(|_| CallFailed)?;
            Ok(Some(serde_json::to_value(&data).map_err(|_| CallFailed)?))
        }
        Call::EffectivePolicy {
            operation,
            existing,
            patch,
            request,
        } => {
            let existing = verification_token_from_row(existing).map_err(|_| CallFailed)?;
            let typed = TypedValue {
                json: serde_json::from_value(request.clone()).map_err(|_| CallFailed)?,
            };
            let body = match operation {
                PolicyOperation::Update => PolicyRequestBody::Update(
                    UpdateKeyRequest::read(&typed).map_err(|_| CallFailed)?,
                ),
                PolicyOperation::Regenerate => PolicyRequestBody::Regenerate(
                    RegenerateKeyRequest::read(&typed).map_err(|_| CallFailed)?,
                ),
                PolicyOperation::Generate => return Err(CallFailed),
            };
            let resolved = match &body {
                PolicyRequestBody::Update(inner) => inner.project(),
                PolicyRequestBody::Regenerate(inner) => inner.project(),
                PolicyRequestBody::Generate(inner) => inner.project(),
            }
            .map_err(|_| CallFailed)?
            .resolved;
            let record = update_policy_request(*operation, &existing, patch, body)
                .map_err(|_| CallFailed)?;
            Ok(Some(json!({
                "operation": record.operation,
                "existing_key": record.existing_key,
                "effective_key": record.effective_key,
                "request": resolved,
            })))
        }
        Call::ModelBudget { budgets } => {
            validate_model_max_budget(budgets.as_ref()).map_err(|_| CallFailed)?;
            Ok(None)
        }
        Call::JsonColumn { column, value } => {
            let decoded = decode_json_string_column(column, value).map_err(|_| CallFailed)?;
            Ok(Some(
                serde_json::to_value(&decoded).map_err(|_| CallFailed)?,
            ))
        }
        Call::VerificationRow { row } => {
            let token = verification_token_from_row(row).map_err(|_| CallFailed)?;
            Ok(Some(serde_json::to_value(&token).map_err(|_| CallFailed)?))
        }
        Call::KeyType { request, columns } => {
            let request = GenerateKeyRequest::read(&TypedValue {
                json: serde_json::from_value(request.clone()).map_err(|_| CallFailed)?,
            })
            .map_err(|_| CallFailed)?;
            let handled = handle_key_type(resolved_key_type(&request), columns.clone());
            Ok(Some(
                serde_json::to_value(&handled).map_err(|_| CallFailed)?,
            ))
        }
        Call::Regenerate { key, body } => {
            let request = RegenerateKeyRequest::read(&TypedValue {
                json: serde_json::from_value(body.clone()).map_err(|_| CallFailed)?,
            })
            .map_err(|_| CallFailed)?;
            let patch = regenerate_as_update_request(key, &request).map_err(|_| CallFailed)?;
            match patch {
                Some(update) => {
                    let resolved = update.project().map_err(|_| CallFailed)?.resolved;
                    Ok(Some(
                        serde_json::to_value(&resolved).map_err(|_| CallFailed)?,
                    ))
                }
                None => Ok(Some(Value::Null)),
            }
        }
        Call::BudgetWindows { budget_limits, now } => {
            let clock = clock_at(now)?;
            let windows =
                generate_budget_windows(budget_limits.as_deref(), clock).map_err(|_| CallFailed)?;
            Ok(Some(
                serde_json::to_value(&windows).map_err(|_| CallFailed)?,
            ))
        }
    }
}

// the oracle always runs the default UTC reset policy; the case freezes the request time
fn clock_at(now: &str) -> Result<KeyClock<'static>, CallFailed> {
    static SETTINGS: OnceLock<BudgetResetSettings> = OnceLock::new();
    static DATABASE: OnceLock<TimeZoneDatabase> = OnceLock::new();
    let parsed = DateTime::parse_from_rfc3339(now)
        .map_err(|_| CallFailed)?
        .with_timezone(&Utc);
    Ok(KeyClock {
        now: parsed,
        reset: SETTINGS.get_or_init(BudgetResetSettings::default),
        database: DATABASE.get_or_init(TimeZoneDatabase::from_env),
    })
}
