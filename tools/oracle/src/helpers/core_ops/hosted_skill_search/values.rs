use betterlite_auth::auth_identity::records::Principal;
use betterlite_core::skills::search::{
    HostedSkillSearchError, SkillSearchError, SkillSearchOutcome, skill_search_result,
};
use betterlite_core::skills::store::StoreError;
use betterlite_db::managed_resource_rows::skills::SkillTable;
use serde_json::{Value, json};

use super::input::{PrincipalFields, RowFields};
use crate::helpers::HelperError;

pub(super) fn invalid_input(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

pub(super) fn store_error(error: &StoreError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "StoreError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

pub(super) fn http_error(error: &SkillSearchError) -> Box<HelperError> {
    match error {
        SkillSearchError::Http {
            status_code,
            detail,
        } => Box::new(HelperError {
            class: "HTTPException".to_owned(),
            status: Some(*status_code),
            error_type: None,
            param: None,
            code: None,
            message: format!("HTTP {status_code}: {detail}"),
        }),
    }
}

pub(super) fn hosted_error(error: &HostedSkillSearchError) -> Box<HelperError> {
    match error {
        HostedSkillSearchError::Store(store) => store_error(store),
        HostedSkillSearchError::Search(search) => http_error(search),
    }
}

pub(super) fn skill_row(fields: &RowFields) -> Result<SkillTable, Box<HelperError>> {
    let value = json!({
        "skill_id": &fields.skill_id,
        "display_title": &fields.display_title,
        "description": &fields.description,
        "instructions": &fields.instructions,
        "created_by": &fields.created_by,
    });
    serde_json::from_value(value).map_err(|error| invalid_input(error.to_string()))
}

pub(super) fn skill_rows(fields: &[RowFields]) -> Result<Vec<SkillTable>, Box<HelperError>> {
    let rows = fields
        .iter()
        .map(skill_row)
        .collect::<Result<Vec<SkillTable>, Box<HelperError>>>()?;
    Ok(rows)
}

pub(super) fn principal_of(fields: &PrincipalFields) -> Result<Principal, Box<HelperError>> {
    let mut teams = Vec::new();
    if let Some(id) = &fields.team_id {
        teams.push(json!({"id": id}));
    }
    let value = json!({
        "principal_type": "human",
        "subject": fields.user_id.as_deref().unwrap_or("scripted"),
        "user": fields.user_id.as_ref().map(|id| json!({"id": id})),
        "organization": fields.org_id.as_ref().map(|id| json!({"id": id})),
        "teams": teams,
        "auth_method": "api_key",
        "credential_ref": {"key_id": &fields.api_key, "token_id": &fields.token},
    });
    serde_json::from_value(value).map_err(|error| invalid_input(error.to_string()))
}

pub(super) fn outcome_value(
    outcome: &SkillSearchOutcome,
    effects: Vec<Value>,
) -> Result<Value, Box<HelperError>> {
    match outcome {
        SkillSearchOutcome::Hits(hits) => {
            let results = hits
                .iter()
                .map(|hit| {
                    serde_json::to_value(skill_search_result(hit))
                        .map_err(|error| invalid_input(error.to_string()))
                })
                .collect::<Result<Vec<Value>, Box<HelperError>>>()?;
            Ok(json!({
                "outcome": "hits",
                "reason": null,
                "hits": results,
                "effects": effects,
            }))
        }
        SkillSearchOutcome::NotConfigured { reason } => Ok(json!({
            "outcome": "not_configured",
            "reason": reason,
            "hits": [],
            "effects": effects,
        })),
        SkillSearchOutcome::EmbeddingFailed { reason } => Ok(json!({
            "outcome": "failed",
            "reason": reason,
            "hits": [],
            "effects": effects,
        })),
        SkillSearchOutcome::UnsupportedProvider { reason } => Ok(json!({
            "outcome": "unsupported_provider",
            "reason": reason,
            "hits": [],
            "effects": effects,
        })),
    }
}
