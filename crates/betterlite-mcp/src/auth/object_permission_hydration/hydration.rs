use super::error::UnloadableEntitlementError;
use super::reads::{ObjectPermission, ObjectPermissionReads};

// the one place a named entitlement becomes UnloadableEntitlementError: a row that names a permission, but cannot read it
pub async fn load_named_object_permission<Reads>(
    reads: &Reads,
    principal: &str,
    object_permission_id: &str,
    requires_fresh_policy: bool,
) -> Result<ObjectPermission, UnloadableEntitlementError<Reads::Error>>
where
    Reads: ObjectPermissionReads,
{
    match reads
        .permission(object_permission_id, requires_fresh_policy)
        .await
    {
        Ok(Some(permission)) => Ok(permission),
        Ok(None) => Err(UnloadableEntitlementError::named(
            principal,
            object_permission_id,
            None,
        )),
        Err(source) => Err(UnloadableEntitlementError::named(
            principal,
            object_permission_id,
            Some(source),
        )),
    }
}
