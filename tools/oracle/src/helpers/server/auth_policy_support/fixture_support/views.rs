use serde_json::{Map, Value, json};

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_db::team_rows::team::TeamFields;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::super::invalid;

// the reconstructed team fields the token carries, as the Python side reads them off the model
pub(crate) fn team_fields(fields: &TeamFields) -> Value {
    json!({
        "team_id": &fields.base.team_id,
        "max_budget": fields.base.max_budget,
        "soft_budget": fields.base.soft_budget,
        "model_max_budget": &fields.model_max_budget.0,
        "spend": fields.spend,
        "tpm_limit": fields.base.tpm_limit,
        "rpm_limit": fields.base.rpm_limit,
        "tpd_limit": fields.base.tpd_limit,
        "blocked": fields.base.blocked,
        "models": &fields.base.models,
        "metadata": &fields.base.metadata,
        "object_permission_id": &fields.object_permission_id,
    })
}

// the token serializes flat, so the identity fields the stage would see come straight off it
pub(crate) fn org_fields(token: &UserAPIKeyAuth) -> HelperResult {
    let serialized = serde_json::to_value(token).or_value_error()?;
    let Value::Object(fields) = serialized else {
        return Err(invalid("the token does not serialize to an object"));
    };
    let mut view = Map::new();
    for name in [
        "org_id",
        "organization_alias",
        "organization_max_budget",
        "organization_tpm_limit",
        "organization_rpm_limit",
        "organization_metadata",
    ] {
        view.insert(
            name.to_owned(),
            fields.get(name).cloned().unwrap_or(Value::Null),
        );
    }
    Ok(Value::Object(view))
}
