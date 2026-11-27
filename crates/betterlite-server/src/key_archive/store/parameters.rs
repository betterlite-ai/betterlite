use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
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
        push_identity(&mut parameters, record);
        push_accounting(&mut parameters, record)?;
        push_lists(&mut parameters, record)?;
        push_owner(&mut parameters, record);
        push_limits(&mut parameters, record)?;
        push_policy(&mut parameters, record);
        push_stamps(&mut parameters, record);
        push_deletion(&mut parameters, record);
    }
    Ok(parameters)
}

fn push_identity(parameters: &mut Vec<Parameter>, record: &DeletedVerificationToken) {
    let token = &record.token;
    let hashed = token
        .token
        .as_ref()
        .map(|secret| secret.expose_secret().to_owned());
    parameters.push(Parameter::Text(record.id.clone()));
    parameters.push(Parameter::Text(hashed));
    parameters.push(Parameter::Text(token.key_name.clone()));
    parameters.push(Parameter::Text(token.key_alias.clone()));
}

fn push_accounting(
    parameters: &mut Vec<Parameter>,
    record: &DeletedVerificationToken,
) -> Result<(), KeyArchiveError> {
    let token = &record.token;
    parameters.push(Parameter::Boolean(token.soft_budget_cooldown.0));
    parameters.push(Parameter::Number(Some(token.spend)));
    parameters.push(Parameter::Number(Some(token.total_spend)));
    parameters.push(Parameter::Timestamp(expiry_instant(
        token.expires.as_ref(),
    )?));
    Ok(())
}

fn push_lists(
    parameters: &mut Vec<Parameter>,
    record: &DeletedVerificationToken,
) -> Result<(), KeyArchiveError> {
    let token = &record.token;
    parameters.push(Parameter::TextArray(text_values(
        Some(token.models.as_slice()),
        "models",
    )?));
    parameters.push(Parameter::JsonPayload(Json(token.aliases.clone())));
    parameters.push(Parameter::JsonPayload(Json(token.config.clone())));
    Ok(())
}

fn push_owner(parameters: &mut Vec<Parameter>, record: &DeletedVerificationToken) {
    let token = &record.token;
    parameters.push(Parameter::Text(token.user_id.clone()));
    parameters.push(Parameter::Text(token.team_id.clone()));
    parameters.push(Parameter::Text(token.agent_id.clone()));
    parameters.push(Parameter::Text(token.project_id.clone()));
    parameters.push(Parameter::JsonPayload(Json(token.permissions.clone())));
    parameters.push(Parameter::Integer(token.max_parallel_requests));
    parameters.push(Parameter::JsonPayload(Json(token.metadata.clone())));
}

fn push_limits(
    parameters: &mut Vec<Parameter>,
    record: &DeletedVerificationToken,
) -> Result<(), KeyArchiveError> {
    let token = &record.token;
    parameters.push(Parameter::OptionalBoolean(token.blocked.map(|flag| flag.0)));
    parameters.push(Parameter::Integer(token.tpm_limit));
    parameters.push(Parameter::Integer(token.rpm_limit));
    parameters.push(Parameter::Integer(token.tpd_limit));
    parameters.push(Parameter::Number(token.max_budget));
    parameters.push(Parameter::Text(token.budget_duration.clone()));
    parameters.push(Parameter::Timestamp(timestamp_instant(
        token.budget_reset_at,
    )));
    let cache_controls = token.allowed_cache_controls.0.as_deref();
    parameters.push(Parameter::TextArray(text_values(
        cache_controls,
        "allowed_cache_controls",
    )?));
    let routes = token.allowed_routes.0.as_deref();
    parameters.push(Parameter::TextArray(text_values(routes, "allowed_routes")?));
    Ok(())
}

fn push_policy(parameters: &mut Vec<Parameter>, record: &DeletedVerificationToken) {
    let token = &record.token;
    parameters.push(Parameter::Text(token.key_type.clone()));
    parameters.push(Parameter::TextArray(token.access_group_ids.clone()));
    parameters.push(Parameter::JsonPayload(Json(token.model_spend.clone())));
    parameters.push(Parameter::JsonPayload(Json(token.model_max_budget.clone())));
    parameters.push(Parameter::JsonFallbacks(Json(
        token.budget_fallbacks.clone(),
    )));
    parameters.push(Parameter::OptionalJsonPayload(
        token.router_settings.clone().map(Json),
    ));
    parameters.push(Parameter::Text(token.budget_id.clone()));
    parameters.push(Parameter::Text(record.organization_id.clone()));
    parameters.push(Parameter::Text(token.object_permission_id.clone()));
}

fn push_stamps(parameters: &mut Vec<Parameter>, record: &DeletedVerificationToken) {
    let token = &record.token;
    parameters.push(Parameter::Timestamp(timestamp_instant(token.created_at)));
    parameters.push(Parameter::Text(token.created_by.clone()));
    parameters.push(Parameter::Timestamp(timestamp_instant(token.updated_at)));
    parameters.push(Parameter::Text(token.updated_by.clone()));
    parameters.push(Parameter::Timestamp(timestamp_instant(
        token.settings_updated_at,
    )));
    parameters.push(Parameter::Timestamp(timestamp_instant(token.last_active)));
    parameters.push(Parameter::Integer(token.rotation_count.0));
    parameters.push(Parameter::OptionalBoolean(
        token.auto_rotate.0.map(|flag| flag.0),
    ));
    parameters.push(Parameter::Text(token.rotation_interval.clone()));
    parameters.push(Parameter::Timestamp(timestamp_instant(
        token.last_rotation_at,
    )));
    parameters.push(Parameter::Timestamp(timestamp_instant(
        token.key_rotation_at,
    )));
}

fn push_deletion(parameters: &mut Vec<Parameter>, record: &DeletedVerificationToken) {
    let actor = record
        .deleted_by_api_key
        .as_ref()
        .map(|secret| secret.expose_secret().to_owned());
    parameters.push(Parameter::Timestamp(timestamp_instant(record.deleted_at)));
    parameters.push(Parameter::Text(record.deleted_by.clone()));
    parameters.push(Parameter::Text(actor));
    parameters.push(Parameter::Text(record.litellm_changed_by.clone()));
}

fn timestamp_instant(timestamp: Option<Timestamp>) -> Option<NaiveDateTime> {
    match timestamp {
        None => None,
        Some(value) => Some(value.utc().naive_utc()),
    }
}

fn expiry_instant(expiry: Option<&Expiry>) -> Result<Option<NaiveDateTime>, KeyArchiveError> {
    let Some(expiry) = expiry else {
        return Ok(None);
    };
    match expiry {
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
    let Some(values) = values else {
        return Ok(None);
    };
    let texts = values
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
