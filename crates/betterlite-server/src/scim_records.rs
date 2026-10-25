pub mod discovery;
pub mod models;
mod tests;

use models::{DecodedScimResource, Group, Patch, ResourceKind, User};
use serde_json::Error;
use std::io::{Error as IoError, ErrorKind};

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
        ResourceKind::Patch => {
            let mut patch = serde_json::from_slice::<Patch>(bytes)?;
            for operation in &mut patch.operations {
                operation.op = operation.op.to_lowercase();
                if !matches!(operation.op.as_str(), "add" | "remove" | "replace") {
                    return Err(Error::io(IoError::new(
                        ErrorKind::InvalidData,
                        "op must be add, remove, or replace",
                    )));
                }
            }
            Ok(DecodedScimResource::Patch(Box::new(patch)))
        }
    }
}
