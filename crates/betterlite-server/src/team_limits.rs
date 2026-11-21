pub mod defaults;
pub mod durations;
pub mod guard;
pub mod memberships;
pub mod model_cap;
pub mod write;

#[path = "team_limits/tests.rs"]
mod tests;

use betterlite_db::budget_permission_rows::JsonData;

use crate::team_request_contracts::update::UpdateTeamRequest;

// pydantic's model_fields_set holds exactly the keys the caller sent, so a present null still counts
pub(crate) fn field_supplied(request: &UpdateTeamRequest, name: &str) -> bool {
    match &request.raw {
        JsonData::Object(fields) => fields.contains_key(name),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::String(_)
        | JsonData::Array(_) => false,
    }
}
