pub mod models;

use models::{DecodedScimResource, Group, Patch, ResourceKind, User};
use serde_json::Error;

pub fn decode_scim_resource(
    bytes: &[u8],
    kind: ResourceKind,
) -> Result<DecodedScimResource, Error> {
    match kind {
        ResourceKind::User => Ok(DecodedScimResource::User(Box::new(
            serde_json::from_slice::<User>(bytes)?,
        ))),
        ResourceKind::Group => Ok(DecodedScimResource::Group(Box::new(
            serde_json::from_slice::<Group>(bytes)?,
        ))),
        ResourceKind::Patch => Ok(DecodedScimResource::Patch(Box::new(
            serde_json::from_slice::<Patch>(bytes)?,
        ))),
    }
}

#[cfg(test)]
mod tests;
