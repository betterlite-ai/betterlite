use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::LiteLLMObjectPermissionTable;

use crate::auth_prefetch::permission::lookup::{
    ObjectPermissionCache, ObjectPermissionSource, PermissionError, PermissionStores,
    get_object_permission,
};
use crate::auth_prefetch::permission::store::ObjectPermissionStore;

// B735's get_object_permission behind one object, so key lookup can own its enrichment.
#[async_trait]
pub trait KeyPermissionLookup: Send + Sync {
    async fn permission(
        &self,
        object_permission_id: &str,
        check_db_only: bool,
    ) -> Result<Option<LiteLLMObjectPermissionTable>, PermissionError>;
}

// The permission cache and the two routed stores the auth state composes.
pub struct PermissionSource {
    pub cache: Arc<dyn ObjectPermissionCache>,
    pub normal: Arc<dyn ObjectPermissionStore>,
    pub writer: Arc<dyn ObjectPermissionStore>,
}

#[async_trait]
impl KeyPermissionLookup for PermissionSource {
    async fn permission(
        &self,
        object_permission_id: &str,
        check_db_only: bool,
    ) -> Result<Option<LiteLLMObjectPermissionTable>, PermissionError> {
        let source = ObjectPermissionSource {
            cache: self.cache.as_ref(),
            stores: Some(PermissionStores {
                normal: self.normal.as_ref(),
                writer: self.writer.as_ref(),
            }),
        };
        get_object_permission(&source, object_permission_id, check_db_only).await
    }
}
