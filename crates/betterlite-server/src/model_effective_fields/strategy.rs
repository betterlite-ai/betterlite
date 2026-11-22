use std::collections::BTreeSet;

use betterlite_db::stored_cipher::DbCipher;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::router_dependencies::STRATEGY_ROUTER_PARAM_FIELDS;
use crate::router_dependencies::config_write::validate_config_write;
use crate::router_dependencies::predicates::carries_complexity_settings;
use crate::router_dependencies::validation::{validate_config_placement, validate_model_write};

use super::decrypted_model;
use super::effective::{effective_complexity_config, effective_model};
use super::error::ModelFieldError;
use super::stored::StoredDeployment;

// Whether B378's member auto-router policy authorized this write; the marker reads nothing else from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteGrant {
    MemberAutoRouter,
    Standard,
}

// What an update does to model_info.member_auto_router.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouterMarker {
    Set,
    Cleared,
    Unchanged,
}

// a write that mangles an auto-router's pseudo-model makes the router drop the deployment at load
pub fn check_strategy_router_write(
    cipher: &DbCipher,
    incoming: Option<&JsonObject>,
    existing: Option<&JsonObject>,
) -> Result<(), ModelFieldError> {
    let Some(params) = incoming else {
        return Ok(());
    };
    // a config is judged only when the write carries one, so a rename is never held by a stored config
    if params
        .get("complexity_router_config")
        .is_some_and(|config| !matches!(config, Json::Null))
    {
        validate_config_write(effective_complexity_config(incoming, existing).as_ref())?;
    }
    let mut present_fields = BTreeSet::new();
    for field in STRATEGY_ROUTER_PARAM_FIELDS {
        let set = [Some(params), existing]
            .into_iter()
            .flatten()
            .any(|source| {
                source
                    .get(field)
                    .is_some_and(|value| !matches!(value, Json::Null))
            });
        if set {
            present_fields.insert(field.to_owned());
        }
    }
    let model = effective_model(cipher, incoming, existing);
    if carries_complexity_settings(model.as_deref(), &present_fields) {
        validate_config_placement(Some(params))?;
    }
    validate_model_write(model.as_deref().unwrap_or_default(), &present_fields)?;
    Ok(())
}

#[must_use]
pub fn member_router_marker(
    cipher: &DbCipher,
    incoming: Option<&JsonObject>,
    existing: &StoredDeployment,
    grant: WriteGrant,
) -> RouterMarker {
    if grant == WriteGrant::MemberAutoRouter {
        return RouterMarker::Set;
    }
    if !matches!(
        existing.model_info.get("member_auto_router"),
        Some(Json::Bool(true))
    ) {
        return RouterMarker::Unchanged;
    }
    let Some(params) = incoming else {
        return RouterMarker::Set;
    };
    let retunes = STRATEGY_ROUTER_PARAM_FIELDS.iter().any(|field| {
        params
            .get(*field)
            .is_some_and(|value| !matches!(value, Json::Null))
    });
    let stored_model = decrypted_model(cipher, existing.litellm_params.get("model"));
    let renames = matches!(
        params.get("model"),
        Some(Json::Text(model)) if Some(model.as_str()) != stored_model.as_deref()
    );
    if retunes || renames {
        return RouterMarker::Cleared;
    }
    RouterMarker::Set
}
