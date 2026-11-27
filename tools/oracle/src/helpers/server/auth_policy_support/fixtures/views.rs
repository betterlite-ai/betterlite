use serde::Serialize;
use serde_json::{Map, Value};

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::super::invalid;

pub(crate) const TEAM_FIELDS: [&str; 12] = [
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
];

pub(crate) const ORG_FIELDS: [&str; 6] = [
    "org_id",
    "organization_alias",
    "organization_max_budget",
    "organization_tpm_limit",
    "organization_rpm_limit",
    "organization_metadata",
];

pub(crate) fn view_fields<T>(value: &T, names: &[&str]) -> HelperResult
where
    T: Serialize,
{
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
