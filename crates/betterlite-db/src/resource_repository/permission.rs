use betterlite_redis::config_sync::table_actions::ConfigSyncTableActions;

use crate::auxiliary_repository::repository::{PrismaClient, PrismaTableRepository, TableBinding};
use crate::postgres_store::errors::DbError;

use super::table::{ResourceActions, ResourceBinding, RowWrite, silent_publisher};

pub struct ObjectPermissionBinding;

impl TableBinding for ObjectPermissionBinding {
    const NAME: &'static str = "litellm_objectpermissiontable";
    const TABLE: &'static str = "LiteLLM_ObjectPermissionTable";
}

impl ResourceBinding for ObjectPermissionBinding {
    const KEY: &'static str = "object_permission_id";
    const ID: Option<&'static str> = Some("object_permission_id");
    const UPDATED_AT: Option<&'static str> = None;
    type Create = RowWrite;
    type Update = RowWrite;
}

pub struct ObjectPermissionRepository<'db> {
    base: PrismaTableRepository<'db, ObjectPermissionBinding>,
}

impl<'db> ObjectPermissionRepository<'db> {
    #[must_use]
    pub fn new(client: Option<PrismaClient<'db>>, use_writer: bool) -> Self {
        let base = PrismaTableRepository::new(client, silent_publisher(), ObjectPermissionBinding);
        Self {
            base: if use_writer { base.with_writer() } else { base },
        }
    }

    pub fn table(
        &self,
    ) -> Result<ConfigSyncTableActions<ResourceActions<'db, ObjectPermissionBinding>>, DbError>
    {
        self.base.table(ResourceActions::new)
    }
}
