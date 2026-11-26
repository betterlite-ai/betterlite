use std::collections::BTreeMap;

use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_error::proxy_error_types::ModelAccessObject;

use crate::auth_model_patterns::decision::{ModelAccessContext, can_object_call_model};
use crate::auth_request_models::RequestedModels;

use super::error::EntityModelAccessError;
use super::grant::entity_grant;

// can_org_access_model: one matcher run, an absent organization reading as no models at all
pub fn can_org_access_model(
    model: &RequestedModels,
    org: Option<&OrganizationTable>,
    aliases: &BTreeMap<String, String>,
    context: &ModelAccessContext<'_>,
) -> Result<(), EntityModelAccessError> {
    EntityModelAccessError::gate(can_object_call_model(
        model,
        &entity_grant(
            org.map_or_else(Vec::new, |row| row.models.clone()),
            Some(aliases.clone()),
            None,
            None,
        ),
        ModelAccessObject::Org,
        context,
        0,
    ))
}
