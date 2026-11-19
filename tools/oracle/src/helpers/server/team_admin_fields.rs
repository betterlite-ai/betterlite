use std::collections::BTreeMap;
use std::collections::BTreeSet;

use betterlite_config::sections::settings::JsonValue;
use betterlite_config::sections::team_admin_fields::{
    EDITABLE_FIELDS_SETTING, SettingWarning, may_edit_member_key_budgets, resolve_editable_fields,
};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::key_fields::decode::TypedValue;
use betterlite_server::key_fields::update::UpdateKeyRequest;
use betterlite_server::team_admin_fields::key as key_policy;
use betterlite_server::team_admin_fields::team as team_policy;
use betterlite_server::team_request_contracts::shared::RequestProjection as _;
use betterlite_server::team_request_contracts::update::UpdateTeamRequest;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;

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
            } => Ok(resolve_answer(general_settings, supported)),
            Input::MemberKeyBudgets { general_settings } => {
                Ok(member_key_budgets_answer(general_settings))
            }
            Input::ChangedTeam { request, existing } => changed_team_answer(request, existing),
            Input::TeamVerdict {
                request,
                existing,
                permitted,
            } => team_verdict_answer(request, existing, permitted),
            Input::TeamRequest {
                request,
                existing,
                permitted,
            } => team_request_answer(request, existing, permitted),
            Input::ChangedKey { request, existing } => changed_key_answer(request, existing),
            Input::KeyVerdict {
                request,
                existing,
                enabled,
            } => key_verdict_answer(request, existing, *enabled),
            Input::KeyRequest {
                request,
                existing,
                enabled,
            } => key_request_answer(request, existing, *enabled),
        }
    }
}

const fn warning_kind(warning: &SettingWarning) -> &'static str {
    match warning {
        SettingWarning::NotAList { .. } => "not_a_list",
        SettingWarning::Unsupported { .. } => "unsupported",
    }
}

fn resolve_answer(general_settings: &BTreeMap<String, JsonValue>, supported: &[String]) -> Value {
    let names: Vec<&str> = supported.iter().map(String::as_str).collect();
    let resolution = resolve_editable_fields(general_settings.get(EDITABLE_FIELDS_SETTING), &names);
    let warnings: Vec<Value> = resolution
        .warnings
        .iter()
        .map(|warning| json!({"kind": warning_kind(warning)}))
        .collect();
    json!({
        "fields": resolution.fields.into_iter().collect::<Vec<_>>(),
        "warnings": warnings,
    })
}

fn member_key_budgets_answer(general_settings: &BTreeMap<String, JsonValue>) -> Value {
    let setting = general_settings.get(EDITABLE_FIELDS_SETTING);
    json!({"allowed": may_edit_member_key_budgets(setting)})
}

fn team_request(document: &JsonData) -> Result<UpdateTeamRequest, Box<HelperError>> {
    UpdateTeamRequest::from_value(document).or_value_error()
}

fn team_row(document: &JsonData) -> Result<LiteLLMTeamTable, Box<HelperError>> {
    LiteLLMTeamTable::try_from(document.clone()).or_value_error()
}

fn key_request(document: &JsonData) -> Result<UpdateKeyRequest, Box<HelperError>> {
    UpdateKeyRequest::read(&TypedValue {
        json: document.clone(),
    })
    .or_value_error()
}

fn key_row(document: &JsonData) -> Result<LiteLLMVerificationToken, Box<HelperError>> {
    let value = serde_json::to_value(document).or_value_error()?;
    serde_json::from_value(value).or_value_error()
}

fn permitted_set(names: &[String]) -> BTreeSet<String> {
    names.iter().cloned().collect()
}

fn changed_team_answer(request: &JsonData, existing: &JsonData) -> HelperResult {
    let changed = team_policy::changed_fields(&team_request(request)?, &team_row(existing)?)
        .or_value_error()?;
    Ok(json!({"changed": changed.into_iter().collect::<Vec<_>>()}))
}

fn team_verdict_answer(
    request: &JsonData,
    existing: &JsonData,
    permitted: &[String],
) -> HelperResult {
    let verdict = team_policy::edit_verdict(
        &team_request(request)?,
        &team_row(existing)?,
        &permitted_set(permitted),
    )
    .or_value_error()?;
    team_verdict_view(verdict)
}

fn team_verdict_view(verdict: team_policy::TeamAdminEditVerdict) -> HelperResult {
    match verdict {
        team_policy::TeamAdminEditVerdict::Allowed { request } => Ok(json!({
            "kind": "allowed",
            "field": null,
            "request": request.supplied().or_value_error()?,
        })),
        team_policy::TeamAdminEditVerdict::Disabled => {
            Ok(json!({"kind": "disabled", "field": null, "request": null}))
        }
        team_policy::TeamAdminEditVerdict::FieldNotPermitted { field } => {
            Ok(json!({"kind": "field_not_permitted", "field": field, "request": null}))
        }
    }
}

fn team_request_answer(
    request: &JsonData,
    existing: &JsonData,
    permitted: &[String],
) -> HelperResult {
    let verdict = team_policy::edit_verdict(
        &team_request(request)?,
        &team_row(existing)?,
        &permitted_set(permitted),
    )
    .or_value_error()?;
    match team_policy::request_or_raise(verdict) {
        Ok(allowed) => Ok(json!({
            "status": null,
            "detail": null,
            "request": allowed.supplied().or_value_error()?,
        })),
        Err(forbidden) => Ok(json!({
            "status": 403,
            "detail": forbidden.to_string(),
            "request": null,
        })),
    }
}

fn changed_key_answer(request: &JsonData, existing: &JsonData) -> HelperResult {
    let changed =
        key_policy::changed_fields(&key_request(request)?, &key_row(existing)?).or_value_error()?;
    Ok(json!({"changed": changed.into_iter().collect::<Vec<_>>()}))
}

fn key_verdict_answer(request: &JsonData, existing: &JsonData, enabled: bool) -> HelperResult {
    let verdict = key_policy::edit_verdict(&key_request(request)?, &key_row(existing)?, enabled)
        .or_value_error()?;
    match verdict {
        key_policy::TeamAdminKeyEditVerdict::Allowed { changed } => Ok(json!({
            "kind": "allowed",
            "field": null,
            "changed": changed.into_iter().collect::<Vec<_>>(),
        })),
        key_policy::TeamAdminKeyEditVerdict::Disabled => {
            Ok(json!({"kind": "disabled", "field": null, "changed": []}))
        }
        key_policy::TeamAdminKeyEditVerdict::FieldNotPermitted { field } => {
            Ok(json!({"kind": "field_not_permitted", "field": field, "changed": []}))
        }
    }
}

fn key_request_answer(request: &JsonData, existing: &JsonData, enabled: bool) -> HelperResult {
    let verdict = key_policy::edit_verdict(&key_request(request)?, &key_row(existing)?, enabled)
        .or_value_error()?;
    match key_policy::request_or_raise(verdict) {
        Ok(()) => Ok(json!({"status": null, "detail": null})),
        Err(forbidden) => Ok(json!({"status": 403, "detail": forbidden.to_string()})),
    }
}
