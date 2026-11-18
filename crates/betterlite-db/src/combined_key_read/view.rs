use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, FixedOffset, NaiveDateTime};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::budget_permission_rows::JsonData;
use crate::identity_rows::Payload;
use crate::identity_rows::key::{Expiry, LiteLLMVerificationToken};
use crate::team_rows::decoder::TeamRowError;
use crate::team_rows::member::Member;

const BUDGET_TABLE_PREFIX: &str = "litellm_budget_table_";

// the same fields B2314 builds for a key view, mirrored below the auth crate that owns that model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CombinedKeyView {
    #[serde(flatten)]
    pub token: LiteLLMVerificationToken,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub team_spend: Option<Decimal>,
    #[serde(default)]
    pub team_alias: Option<String>,
    #[serde(default)]
    pub team_tpm_limit: Option<i64>,
    #[serde(default)]
    pub team_rpm_limit: Option<i64>,
    #[serde(default)]
    pub team_tpd_limit: Option<i64>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub team_max_budget: Option<Decimal>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub team_soft_budget: Option<Decimal>,
    #[serde(default)]
    pub team_model_max_budget: Option<Payload>,
    #[serde(default)]
    pub team_models: Vec<JsonData>,
    #[serde(default)]
    pub team_blocked: bool,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub soft_budget: Option<Decimal>,
    #[serde(default)]
    pub team_model_aliases: Option<Payload>,
    #[serde(default)]
    pub team_member: Option<Member>,
    #[serde(default)]
    pub team_metadata: Option<Payload>,
    #[serde(default)]
    pub team_object_permission_id: Option<String>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub team_member_spend: Option<Decimal>,
    #[serde(default)]
    pub team_member_tpm_limit: Option<i64>,
    #[serde(default)]
    pub team_member_rpm_limit: Option<i64>,
    #[serde(default)]
    pub end_user_id: Option<String>,
    #[serde(default)]
    pub end_user_tpm_limit: Option<i64>,
    #[serde(default)]
    pub end_user_rpm_limit: Option<i64>,
    #[serde(default)]
    pub end_user_tpd_limit: Option<i64>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub end_user_max_budget: Option<Decimal>,
    #[serde(default)]
    pub end_user_model_max_budget: Option<Payload>,
    #[serde(default)]
    pub organization_alias: Option<String>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub organization_max_budget: Option<Decimal>,
    #[serde(default)]
    pub organization_tpm_limit: Option<i64>,
    #[serde(default)]
    pub organization_rpm_limit: Option<i64>,
    #[serde(default)]
    pub organization_metadata: Option<Payload>,
    #[serde(default)]
    pub project_alias: Option<String>,
    #[serde(default)]
    pub project_metadata: Option<Payload>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub last_refreshed_at: Option<Decimal>,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum ViewError {
    #[error("failed to decode a verification token view")]
    Json(#[source] Arc<serde_json::Error>),
    #[error("the stored key row is not a JSON object")]
    NotAnObject,
    #[error("a team member entry is not an object")]
    MemberShape,
    #[error("a team member entry is invalid: {0}")]
    Member(#[source] Arc<TeamRowError>),
}

impl CombinedKeyView {
    pub fn construct(
        mut fields: BTreeMap<String, JsonData>,
        refreshed_at: Decimal,
    ) -> Result<Self, ViewError> {
        drop_null_columns(&mut fields);
        let member = select_team_member(&fields)?;
        apply_budget_aliases(&mut fields);
        stringify_end_user_id(&mut fields);
        promote_key_organization(&mut fields);
        let text = serde_json::to_string(&JsonData::Object(fields))
            .map_err(|error| ViewError::Json(Arc::new(error)))?;
        let mut view: Self =
            serde_json::from_str(&text).map_err(|error| ViewError::Json(Arc::new(error)))?;
        view.team_member = member;
        view.last_refreshed_at = Some(refreshed_at);
        if let Some(expires) = view.token.expires.take() {
            view.token.expires = Some(expires_isoformat(expires));
        }
        Ok(view)
    }
}

// the flattened token decodes through serde's buffered Content, where a null option reaches a visitor that rejects it, and LiteLLM reads null as the field's default
fn drop_null_columns(fields: &mut BTreeMap<String, JsonData>) {
    fields.retain(|_, value| !matches!(value, JsonData::Null));
}

// LiteLLM walks every role entry and keeps the last whose user_id equals the key's, so an email alone never matches
fn select_team_member(fields: &BTreeMap<String, JsonData>) -> Result<Option<Member>, ViewError> {
    let Some(JsonData::String(key_user)) = fields.get("user_id") else {
        return Ok(None);
    };
    let Some(JsonData::Array(entries)) = fields.get("team_members_with_roles") else {
        return Ok(None);
    };
    let mut selected = None;
    for entry in entries {
        let JsonData::Object(entry_fields) = entry else {
            return Err(ViewError::MemberShape);
        };
        if !matches!(
            entry_fields.get("user_id"),
            Some(JsonData::String(user)) if user == key_user
        ) {
            continue;
        }
        let member =
            Member::try_from(entry.clone()).map_err(|error| ViewError::Member(Arc::new(error)))?;
        selected = Some(member);
    }
    Ok(selected)
}

fn apply_budget_aliases(fields: &mut BTreeMap<String, JsonData>) {
    let originals: Vec<(String, JsonData)> = fields
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    for (key, value) in originals {
        let Some(alias) = key.strip_prefix(BUDGET_TABLE_PREFIX) else {
            continue;
        };
        if matches!(value, JsonData::Null) {
            continue;
        }
        let target = alias.to_owned();
        let wins = match fields.get(&target) {
            None | Some(JsonData::Null) => true,
            Some(JsonData::Object(entry)) => target == "model_max_budget" && entry.is_empty(),
            Some(
                JsonData::Bool(_) | JsonData::Number(_) | JsonData::String(_) | JsonData::Array(_),
            ) => false,
        };
        if wins {
            fields.insert(target, value);
        }
    }
}

// Python's str() of a bool is True/False and of an int is its digits
fn stringify_end_user_id(fields: &mut BTreeMap<String, JsonData>) {
    let Some(value) = fields.get("end_user_id") else {
        return;
    };
    let text = match value {
        JsonData::Bool(flag) => String::from(if *flag { "True" } else { "False" }),
        JsonData::Number(number) if !number.is_f64() => number.to_string(),
        JsonData::Number(_)
        | JsonData::Null
        | JsonData::String(_)
        | JsonData::Array(_)
        | JsonData::Object(_) => return,
    };
    fields.insert("end_user_id".to_owned(), JsonData::String(text));
}

// the key's own organization wins over the team organization the query also selected
fn promote_key_organization(fields: &mut BTreeMap<String, JsonData>) {
    if matches!(fields.get("organization_id"), None | Some(JsonData::Null)) {
        return;
    }
    if let Some(value) = fields.remove("organization_id") {
        fields.insert("org_id".to_owned(), value);
    }
}

fn expires_isoformat(expires: Expiry) -> Expiry {
    match expires {
        Expiry::Text(text) => {
            if let Ok(value) = DateTime::parse_from_rfc3339(&text) {
                return Expiry::Text(format_naive(value.naive_local(), Some(*value.offset())));
            }
            for pattern in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f"] {
                if let Ok(value) = NaiveDateTime::parse_from_str(&text, pattern) {
                    return Expiry::Text(format_naive(value, None));
                }
            }
            Expiry::Text(text)
        }
        Expiry::DateTime(value) => {
            Expiry::Text(format_naive(value.naive_local(), Some(*value.offset())))
        }
    }
}

fn format_naive(value: NaiveDateTime, offset: Option<FixedOffset>) -> String {
    let base = value.format("%Y-%m-%dT%H:%M:%S").to_string();
    let micros = value.and_utc().timestamp_subsec_micros();
    let fraction = if micros == 0 {
        String::new()
    } else {
        format!(".{micros:06}")
    };
    offset.map_or_else(
        || format!("{base}{fraction}"),
        |zone| format!("{base}{fraction}{zone}"),
    )
}

#[must_use]
pub fn epoch_decimal(now: DateTime<chrono::Utc>) -> Decimal {
    let nanos = Decimal::from(now.timestamp_subsec_nanos());
    let fraction = nanos
        .checked_div(Decimal::from(1_000_000_000_i64))
        .unwrap_or_default();
    Decimal::from(now.timestamp()).saturating_add(fraction)
}
