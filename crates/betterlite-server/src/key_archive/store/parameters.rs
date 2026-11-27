use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;
use betterlite_db::identity_rows::key::{DeletedVerificationToken, Expiry};
use chrono::{DateTime, NaiveDateTime};
use rust_decimal::Decimal;
use secrecy::ExposeSecret as _;
use tokio_postgres::types::{Json, ToSql};

use crate::key_archive::KeyArchiveError;

use super::sql::COLUMNS;

pub(super) enum Parameter {
    Text(Option<String>),
    Boolean(bool),
    OptionalBoolean(Option<bool>),
    Integer(Option<i64>),
    Number(Option<Decimal>),
    Timestamp(Option<NaiveDateTime>),
    TextArray(Option<Vec<String>>),
    JsonPayload(Json<Payload>),
    JsonFallbacks(Json<BTreeMap<String, Vec<String>>>),
    OptionalJsonPayload(Option<Json<Payload>>),
}

impl Parameter {
    pub(super) fn bind(&self) -> &(dyn ToSql + Sync) {
        match self {
            Self::Text(value) => value,
            Self::Boolean(value) => value,
            Self::OptionalBoolean(value) => value,
            Self::Integer(value) => value,
            Self::Number(value) => value,
            Self::Timestamp(value) => value,
            Self::TextArray(value) => value,
            Self::JsonPayload(value) => value,
            Self::JsonFallbacks(value) => value,
            Self::OptionalJsonPayload(value) => value,
        }
    }
}

pub(super) fn archived_parameters(
    records: &[DeletedVerificationToken],
) -> Result<Vec<Parameter>, KeyArchiveError> {
    let mut parameters = Vec::with_capacity(records.len().saturating_mul(COLUMNS.len()));
    for record in records {
        parameters.extend(text_parameters(record));
        parameters.extend(boolean_parameters(record));
        parameters.extend(integer_parameters(record));
        parameters.extend(number_parameters(record));
        parameters.extend(timestamp_parameters(record)?);
        parameters.extend(array_parameters(record)?);
        parameters.extend(json_parameters(record));
    }
    Ok(parameters)
}

fn text_parameters(record: &DeletedVerificationToken) -> [Parameter; 19] {
    let token = &record.token;
    let hashed = token
        .token
        .as_ref()
        .map(|secret| secret.expose_secret().to_owned());
    let actor = record
        .deleted_by_api_key
        .as_ref()
        .map(|secret| secret.expose_secret().to_owned());
    [
        Parameter::Text(record.id.clone()),
        Parameter::Text(hashed),
        Parameter::Text(token.key_name.clone()),
        Parameter::Text(token.key_alias.clone()),
        Parameter::Text(token.user_id.clone()),
        Parameter::Text(token.team_id.clone()),
        Parameter::Text(token.agent_id.clone()),
        Parameter::Text(token.project_id.clone()),
        Parameter::Text(token.budget_duration.clone()),
        Parameter::Text(token.key_type.clone()),
        Parameter::Text(token.budget_id.clone()),
        Parameter::Text(record.organization_id.clone()),
        Parameter::Text(token.object_permission_id.clone()),
        Parameter::Text(token.created_by.clone()),
        Parameter::Text(token.updated_by.clone()),
        Parameter::Text(token.rotation_interval.clone()),
        Parameter::Text(record.deleted_by.clone()),
        Parameter::Text(actor),
        Parameter::Text(record.litellm_changed_by.clone()),
    ]
}

fn boolean_parameters(record: &DeletedVerificationToken) -> [Parameter; 3] {
    [
        Parameter::Boolean(record.token.soft_budget_cooldown.0),
        Parameter::OptionalBoolean(record.token.blocked.map(|flag| flag.0)),
        Parameter::OptionalBoolean(record.token.auto_rotate.0.map(|flag| flag.0)),
    ]
}

const fn integer_parameters(record: &DeletedVerificationToken) -> [Parameter; 5] {
    [
        Parameter::Integer(record.token.max_parallel_requests),
        Parameter::Integer(record.token.tpm_limit),
        Parameter::Integer(record.token.rpm_limit),
        Parameter::Integer(record.token.tpd_limit),
        Parameter::Integer(record.token.rotation_count.0),
    ]
}

const fn number_parameters(record: &DeletedVerificationToken) -> [Parameter; 3] {
    [
        Parameter::Number(Some(record.token.spend)),
        Parameter::Number(Some(record.token.total_spend)),
        Parameter::Number(record.token.max_budget),
    ]
}

fn timestamp_parameters(
    record: &DeletedVerificationToken,
) -> Result<Vec<Parameter>, KeyArchiveError> {
    let token = &record.token;
    let stamps = [
        token.budget_reset_at,
        token.created_at,
        token.updated_at,
        token.settings_updated_at,
        token.last_active,
        token.last_rotation_at,
        token.key_rotation_at,
        record.deleted_at,
    ];
    let mut parameters = vec![Parameter::Timestamp(expiry_instant(
        token.expires.as_ref(),
    )?)];
    parameters.extend(
        stamps.map(|stamp| Parameter::Timestamp(stamp.map(|value| value.utc().naive_utc()))),
    );
    Ok(parameters)
}

fn array_parameters(record: &DeletedVerificationToken) -> Result<[Parameter; 4], KeyArchiveError> {
    let token = &record.token;
    let cache_controls = token.allowed_cache_controls.0.as_deref();
    let routes = token.allowed_routes.0.as_deref();
    let models = text_values(Some(token.models.as_slice()), "models")?;
    let controls = text_values(cache_controls, "allowed_cache_controls")?;
    let allowed = text_values(routes, "allowed_routes")?;
    let groups = token.access_group_ids.clone();
    Ok([
        Parameter::TextArray(models),
        Parameter::TextArray(controls),
        Parameter::TextArray(allowed),
        Parameter::TextArray(groups),
    ])
}

fn json_parameters(record: &DeletedVerificationToken) -> [Parameter; 8] {
    let token = &record.token;
    [
        Parameter::JsonPayload(Json(token.aliases.clone())),
        Parameter::JsonPayload(Json(token.config.clone())),
        Parameter::JsonPayload(Json(token.permissions.clone())),
        Parameter::JsonPayload(Json(token.metadata.clone())),
        Parameter::JsonPayload(Json(token.model_spend.clone())),
        Parameter::JsonPayload(Json(token.model_max_budget.clone())),
        Parameter::JsonFallbacks(Json(token.budget_fallbacks.clone())),
        Parameter::OptionalJsonPayload(token.router_settings.clone().map(Json)),
    ]
}

fn expiry_instant(expiry: Option<&Expiry>) -> Result<Option<NaiveDateTime>, KeyArchiveError> {
    let Some(given) = expiry else {
        return Ok(None);
    };
    match given {
        Expiry::DateTime(value) => Ok(Some(value.naive_utc())),
        Expiry::Text(text) => parse_expiry_instant(text)
            .map(Some)
            .ok_or(KeyArchiveError::InvalidTimestamp { column: "expires" }),
    }
}

fn parse_expiry_instant(text: &str) -> Option<NaiveDateTime> {
    if let Ok(value) = DateTime::parse_from_rfc3339(text) {
        return Some(value.naive_utc());
    }
    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f").ok()
}

fn text_values(
    values: Option<&[JsonData]>,
    column: &'static str,
) -> Result<Option<Vec<String>>, KeyArchiveError> {
    let Some(given) = values else {
        return Ok(None);
    };
    let texts = given
        .iter()
        .map(|value| match value {
            JsonData::String(text) => Ok(text.clone()),
            JsonData::Null
            | JsonData::Bool(_)
            | JsonData::Number(_)
            | JsonData::Array(_)
            | JsonData::Object(_) => Err(KeyArchiveError::NonTextValue { column }),
        })
        .collect::<Result<Vec<String>, KeyArchiveError>>()?;
    Ok(Some(texts))
}
