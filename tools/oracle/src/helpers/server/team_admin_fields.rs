use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;
use betterlite_config::sections::team_admin_fields::{
    EDITABLE_FIELDS_SETTING, SettingWarning, may_edit_member_key_budgets, resolve_editable_fields,
};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken as KeyRow;
use betterlite_db::team_rows::team::LiteLLMTeamTable as TeamRow;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::key_fields::decode::TypedValue;
use betterlite_server::key_fields::update::UpdateKeyRequest;
use betterlite_server::team_admin_fields::error::Forbidden;
use betterlite_server::team_admin_fields::key;
use betterlite_server::team_admin_fields::key::TeamAdminKeyEditVerdict;
use betterlite_server::team_admin_fields::key::TeamAdminKeyEditVerdict::{
    Allowed as KeyAllowed, Disabled as KeyDisabled, FieldNotPermitted as KeyFieldNotPermitted,
};
use betterlite_server::team_admin_fields::team;
use betterlite_server::team_admin_fields::team::TeamAdminEditVerdict;
use betterlite_server::team_admin_fields::team::TeamAdminEditVerdict::{
    Allowed, Disabled, FieldNotPermitted,
};
use betterlite_server::team_request_contracts::shared::RequestProjection as _;
use betterlite_server::team_request_contracts::update::UpdateTeamRequest;
use serde::Deserialize;
use serde_json::Value;
use serde_json::from_value;
use serde_json::json;
use serde_json::to_value;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Resolve {
        general_settings: BTreeMap<String, JsonValue>,
        supported: Vec<String>,
    },
    MemberKeyBudgets {
        general_settings: BTreeMap<String, JsonValue>,
    },
    ChangedTeam {
        request: JsonData,
        existing: JsonData,
    },
    TeamVerdict {
        request: JsonData,
        existing: JsonData,
        permitted: Vec<String>,
    },
    TeamRequest {
        request: JsonData,
        existing: JsonData,
        permitted: Vec<String>,
    },
    ChangedKey {
        request: JsonData,
        existing: JsonData,
    },
    KeyVerdict {
        request: JsonData,
        existing: JsonData,
        enabled: bool,
    },
    KeyRequest {
        request: JsonData,
        existing: JsonData,
        enabled: bool,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Resolve {
                general_settings,
                supported,
            } => Ok(resolve(general_settings, supported)),
            Input::MemberKeyBudgets { general_settings } => Ok(
                json!({"allowed": may_edit_member_key_budgets(general_settings.get(EDITABLE_FIELDS_SETTING))}),
            ),
            Input::ChangedTeam { request, existing } => changed_team(request, existing),
            Input::ChangedKey { request, existing } => changed_key(request, existing),
            Input::TeamVerdict {
                request,
                existing,
                permitted,
            } => team_verdict(request, existing, permitted),
            Input::TeamRequest {
                request,
                existing,
                permitted,
            } => team_request(request, existing, permitted),
            Input::KeyVerdict {
                request,
                existing,
                enabled,
            } => key_verdict(request, existing, *enabled),
            Input::KeyRequest {
                request,
                existing,
                enabled,
            } => key_request(request, existing, *enabled),
        }
    }
}

const fn warning_kind(warning: &SettingWarning) -> &'static str {
    match warning {
        SettingWarning::NotAList { .. } => "not_a_list",
        SettingWarning::Unsupported { .. } => "unsupported",
    }
}

fn resolve(general_settings: &BTreeMap<String, JsonValue>, supported: &[String]) -> Value {
    let names: Vec<&str> = supported.iter().map(String::as_str).collect();
    let resolved = resolve_editable_fields(general_settings.get(EDITABLE_FIELDS_SETTING), &names);
    let kinds: Vec<&str> = resolved.warnings.iter().map(warning_kind).collect();
    json!({"fields": resolved.fields.into_iter().collect::<Vec<_>>(), "warnings": kinds.into_iter().map(|kind| json!({"kind": kind})).collect::<Vec<_>>()})
}

fn changed_team(request: &JsonData, existing: &JsonData) -> HelperResult {
    let submitted = UpdateTeamRequest::from_value(request).or_value_error()?;
    let stored = TeamRow::try_from(existing.clone()).or_value_error()?;
    let changed = team::changed_fields(&submitted, &stored).or_value_error()?;
    Ok(json!({"changed": changed.into_iter().collect::<Vec<_>>()}))
}

fn decode_key(request: &JsonData) -> Result<UpdateKeyRequest, Box<HelperError>> {
    let value = TypedValue {
        json: request.clone(),
    };
    let submitted = UpdateKeyRequest::read(&value).or_value_error()?;
    Ok(submitted)
}

fn changed_key(request: &JsonData, existing: &JsonData) -> HelperResult {
    let submitted = decode_key(request)?;
    let stored = from_value::<KeyRow>(to_value(existing).or_value_error()?).or_value_error()?;
    let changed = key::changed_fields(&submitted, &stored).or_value_error()?;
    Ok(json!({"changed": changed.into_iter().collect::<Vec<_>>()}))
}

fn team_verdict(request: &JsonData, existing: &JsonData, permitted: &[String]) -> HelperResult {
    let submitted = UpdateTeamRequest::from_value(request).or_value_error()?;
    let stored = TeamRow::try_from(existing.clone()).or_value_error()?;
    let verdict = team::edit_verdict(&submitted, &stored, &permitted.iter().cloned().collect())
        .or_value_error()?;
    Ok(team_verdict_view(&verdict))
}

fn team_request(request: &JsonData, existing: &JsonData, permitted: &[String]) -> HelperResult {
    let submitted = UpdateTeamRequest::from_value(request).or_value_error()?;
    let stored = TeamRow::try_from(existing.clone()).or_value_error()?;
    let verdict = team::edit_verdict(&submitted, &stored, &permitted.iter().cloned().collect())
        .or_value_error()?;
    Ok(team_request_view(team::request_or_raise(verdict)))
}

fn key_verdict(request: &JsonData, existing: &JsonData, enabled: bool) -> HelperResult {
    let submitted = decode_key(request)?;
    let stored = from_value::<KeyRow>(to_value(existing).or_value_error()?).or_value_error()?;
    let verdict = key::edit_verdict(&submitted, &stored, enabled).or_value_error()?;
    Ok(key_verdict_view(&verdict))
}

fn key_request(request: &JsonData, existing: &JsonData, enabled: bool) -> HelperResult {
    let submitted = decode_key(request)?;
    let stored = from_value::<KeyRow>(to_value(existing).or_value_error()?).or_value_error()?;
    let verdict = key::edit_verdict(&submitted, &stored, enabled).or_value_error()?;
    Ok(key_request_view(key::request_or_raise(verdict)))
}

fn team_verdict_view(verdict: &TeamAdminEditVerdict) -> HelperResult {
    let (kind, field, request) = match verdict {
        Allowed { request } => ("allowed", None, Some(request.supplied().or_value_error()?)),
        Disabled => ("disabled", None, None),
        FieldNotPermitted { field } => ("field_not_permitted", Some(field.as_str()), None),
    };
    Ok(json!({"kind": kind, "field": field, "request": request}))
}

fn team_request_view(outcome: Result<UpdateTeamRequest, Forbidden>) -> HelperResult {
    let (status, detail, request) = match outcome {
        Ok(allowed) => (None, None, Some(allowed.supplied().or_value_error()?)),
        Err(forbidden) => (Some(403_u16), Some(forbidden.to_string()), None),
    };
    Ok(json!({"status": status, "detail": detail, "request": request}))
}

fn key_verdict_view(verdict: &TeamAdminKeyEditVerdict) -> Value {
    let (kind, field, changed) = match verdict {
        KeyAllowed { changed } => ("allowed", None, changed.iter().collect::<Vec<_>>()),
        KeyDisabled => ("disabled", None, Vec::new()),
        KeyFieldNotPermitted { field } => ("field_not_permitted", Some(field.as_str()), Vec::new()),
    };
    json!({"kind": kind, "field": field, "changed": changed})
}

fn key_request_view(outcome: Result<(), Forbidden>) -> Value {
    let (status, detail) = match outcome {
        Ok(()) => (None, None),
        Err(forbidden) => (Some(403_u16), Some(forbidden.to_string())),
    };
    json!({"status": status, "detail": detail})
}
