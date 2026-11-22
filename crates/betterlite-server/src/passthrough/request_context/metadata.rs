use betterlite_auth::auth_request_models::sources::{DispatchContext, is_pass_through_dispatch};
use betterlite_core::api::request_destination::RequestData;
use betterlite_core::api::request_principal_metadata::Metadata;
use betterlite_core::api::request_principal_metadata::principal::logged_api_key;
use betterlite_models::decisions::Json;

use super::KEPT_OUT_METADATA_KEYS;
// ast-grep-ignore: no-long-names
use super::MODEL_ACCESS_GROUP_METADATA_KEY;
use super::{PassThroughContextError, PassThroughKey};
// ast-grep-ignore: no-long-names
use super::SESSION_ID_OMITTED_METADATA_KEY;

const MODEL_MAX_BUDGET_KEY: &str = "user_api_key_model_max_budget";
const TEAM_MODEL_MAX_BUDGET_KEY: &str = "user_api_key_team_model_max_budget";
const USER_MODEL_MAX_BUDGET_KEY: &str = "user_api_key_user_model_max_budget";
const END_USER_MODEL_MAX_BUDGET_KEY: &str = "user_api_key_end_user_model_max_budget";
const USER_API_KEY_KEY: &str = "user_api_key";
const PARENT_SPAN_KEY: &str = "litellm_parent_otel_span";
const BUDGET_RESERVATION_KEY: &str = "user_api_key_budget_reservation";

pub(super) fn client_metadata(body: &RequestData) -> Result<Metadata, PassThroughContextError> {
    let mut merged = Metadata::new();
    for field in KEPT_OUT_METADATA_KEYS {
        match body.get(field) {
            None | Some(Json::Null) => {}
            Some(Json::Object(values)) => merged.extend(values.clone()),
            Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
                return Err(PassThroughContextError::MetadataNotObject { field });
            }
        }
    }
    Ok(merged)
}

// the authenticated facts land after the client merge so a request body cannot clobber them
pub(super) fn stamp_internal_keys(
    metadata: &mut Metadata,
    user_api_key_dict: &PassThroughKey,
    dispatch: &DispatchContext<'_>,
) {
    stamp_identity_keys(metadata, user_api_key_dict);
    if !is_pass_through_dispatch(dispatch) {
        stamp_budget_keys(metadata, user_api_key_dict);
    }
}

fn stamp_identity_keys(metadata: &mut Metadata, user_api_key_dict: &PassThroughKey) {
    let principal = &user_api_key_dict.principal;
    metadata.shift_remove(SESSION_ID_OMITTED_METADATA_KEY); // ast-grep-ignore: no-long-names
    metadata.insert(
        USER_API_KEY_KEY.to_owned(),
        logged_api_key(principal).map_or(Json::Null, |key| Json::Text(key.to_owned())),
    );
    metadata.insert(
        PARENT_SPAN_KEY.to_owned(),
        user_api_key_dict.parent_otel_span.clone(),
    );
    metadata.insert(
        BUDGET_RESERVATION_KEY.to_owned(),
        principal
            .budget_reservation
            .as_ref()
            .map_or(Json::Null, |object| Json::Object(object.clone())),
    );
    metadata.insert(
        MODEL_ACCESS_GROUP_METADATA_KEY.to_owned(), // ast-grep-ignore: no-long-names
        Json::List(
            principal
                .matched_model_access_groups
                .iter()
                .cloned()
                .map(Json::Text)
                .collect(),
        ),
    );
}

// only the built-in provider routes charge the per-model counters a user-defined route cannot reach
fn stamp_budget_keys(metadata: &mut Metadata, user_api_key_dict: &PassThroughKey) {
    for (name, budget) in [
        (MODEL_MAX_BUDGET_KEY, &user_api_key_dict.model_max_budget),
        (
            TEAM_MODEL_MAX_BUDGET_KEY,
            &user_api_key_dict.team_model_max_budget,
        ),
        (
            USER_MODEL_MAX_BUDGET_KEY,
            &user_api_key_dict.user_model_max_budget,
        ),
        (
            END_USER_MODEL_MAX_BUDGET_KEY,
            &user_api_key_dict.end_user_model_max_budget,
        ),
    ] {
        metadata.insert(
            name.to_owned(),
            budget
                .as_ref()
                .map_or(Json::Null, |object| Json::Object(object.clone())),
        );
    }
}
