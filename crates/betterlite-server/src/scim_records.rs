mod models;

pub use models::{
    DecodedScimResource, Group, Patch, PatchOperation, ResourceKind, ScimEnterpriseUser,
    ScimMember, ScimMultiValuedAttribute, ScimResource, ScimResourceType, ScimSchema,
    ScimSchemaAttribute, ScimSchemaExtension, User, UserEmail, UserGroup, UserManager, UserName,
};

use serde_json::Error;

pub fn decode_scim_resource(
    bytes: &[u8],
    kind: ResourceKind,
) -> Result<DecodedScimResource, Error> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    match kind {
        ResourceKind::User => Ok(DecodedScimResource::User(serde_json::from_value(value)?)),
        ResourceKind::Group => Ok(DecodedScimResource::Group(serde_json::from_value(value)?)),
        ResourceKind::Patch => Ok(DecodedScimResource::Patch(serde_json::from_value(value)?)),
    }
}

#[cfg(test)]
mod tests;
