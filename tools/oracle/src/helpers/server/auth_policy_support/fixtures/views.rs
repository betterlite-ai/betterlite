use serde::Serialize;
use serde_json::{Map, Value};

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_db::team_rows::team::TeamFields;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::super::invalid;

// the reconstructed team fields the token carries, as the Python side reads them off the model
pub(crate) fn team_fields(fields: &TeamFields) -> HelperResult {
    view_fields(
        fields,
        &[
            "team_id",
            "max_budget",
            "soft_budget",
            "model_max_budget",
            "spend",
            "tpm_limit",
            "rpm_limit",
            "tpd_limit",
            "blocked",
            "models",
            "metadata",
            "object_permission_id",
        ],
    )
}

// the token serializes flat, so the identity fields the stage would see come straight off it
pub(crate) fn org_fields(token: &UserAPIKeyAuth) -> HelperResult {
    view_fields(
        token,
        &[
            "org_id",
            "organization_alias",
            "organization_max_budget",
            "organization_tpm_limit",
            "organization_rpm_limit",
            "organization_metadata",
        ],
    )
}

fn view_fields<T: Serialize>(value: &T, names: &[&str]) -> HelperResult {
    let serialized = serde_json::to_value(value).or_value_error()?;
    let Value::Object(fields) = serialized else {
        return Err(invalid("the model does not serialize to an object"));
    };
    let mut view = Map::new();
    for name in names {
        view.insert(
            (*name).to_owned(),
            fields.get(*name).cloned().unwrap_or(Value::Null),
        );
    }
    Ok(Value::Object(view))
}
