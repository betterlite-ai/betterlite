pub mod discovery;
pub mod models;
mod tests;
use models::{DecodedScimResource, Group, Patch, ResourceKind, User};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ScimRecordError {
    #[error("invalid SCIM JSON: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("unsupported SCIM patch operation: {op}")]
    UnsupportedOp { op: String },
}
pub fn decode_scim_resource(
    bytes: &[u8],
    kind: ResourceKind,
) -> Result<DecodedScimResource, ScimRecordError> {
    match kind {
        ResourceKind::User => Ok(DecodedScimResource::User(Box::new(
            serde_json::from_slice::<User>(bytes)
                .map_err(|source| ScimRecordError::Json { source })?,
        ))),
        ResourceKind::Group => Ok(DecodedScimResource::Group(Box::new(
            serde_json::from_slice::<Group>(bytes)
                .map_err(|source| ScimRecordError::Json { source })?,
        ))),
        ResourceKind::Patch => {
            let mut patch = serde_json::from_slice::<Patch>(bytes)
                .map_err(|source| ScimRecordError::Json { source })?;
            for operation in &mut patch.operations {
                operation.op = operation.op.to_lowercase();
                if !matches!(operation.op.as_str(), "add" | "remove" | "replace") {
                    return Err(ScimRecordError::UnsupportedOp {
                        op: operation.op.clone(),
                    });
                }
            }
            Ok(DecodedScimResource::Patch(Box::new(patch)))
        }
    }
}
