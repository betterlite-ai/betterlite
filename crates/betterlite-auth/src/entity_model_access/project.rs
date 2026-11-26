use std::collections::BTreeMap;

use betterlite_db::resource_rows::project::ProjectTable;
use betterlite_error::proxy_error_types::ModelAccessObject;

use crate::auth_model_patterns::decision::{ModelAccessContext, can_object_call_model};
use crate::auth_request_models::RequestedModels;

use super::error::EntityModelAccessError;
use super::grant::entity_grant;

// can_project_access_model: one matcher run with the key aliases and no team aliases
pub fn can_project_access_model(
    model: &RequestedModels,
    project: Option<&ProjectTable>,
    alias_targets: &BTreeMap<String, String>,
    context: &ModelAccessContext<'_>,
) -> Result<(), EntityModelAccessError> {
    EntityModelAccessError::gate(can_object_call_model(
        model,
        &entity_grant(
            project.map_or_else(Vec::new, |row| row.models.clone()),
            None,
            Some(alias_targets.clone()),
            None,
        ),
        ModelAccessObject::Project,
        context,
        0,
    ))
}
