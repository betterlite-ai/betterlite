use betterlite_redis::config_sync::publish::redis_config_change_publisher;
use betterlite_redis::config_sync::table_actions::ConfigSyncTableActions;

use crate::auxiliary_repository::bindings::ObjectPermissionTable;
use crate::auxiliary_repository::repository::{PrismaClient, PrismaTableRepository};
use crate::postgres_store::errors::DbError;

use super::table::{ResourceActions, ResourceBinding, RowWrite};

impl ResourceBinding for ObjectPermissionTable {
    const KEY: &'static str = "object_permission_id";
    const ID: Option<&'static str> = Some("object_permission_id");
    const UPDATED_AT: Option<&'static str> = None;
    type Create = RowWrite;
    type Update = RowWrite;
}

pub struct ObjectPermissionRepository<'db> {
    base: PrismaTableRepository<'db, ObjectPermissionTable>,
}

impl<'db> ObjectPermissionRepository<'db> {
    #[must_use]
    pub fn new(client: Option<PrismaClient<'db>>, use_writer: bool) -> Self {
        let base = PrismaTableRepository::new(
            client,
            redis_config_change_publisher(None),
            ObjectPermissionTable,
        );
        Self {
            base: if use_writer { base.with_writer() } else { base },
        }
    }

    // ast-grep-ignore: no-one-line-functions - LiteLLM's table property builds the same actions
    pub fn table(
        &self,
    ) -> Result<ConfigSyncTableActions<ResourceActions<'db, ObjectPermissionTable>>, DbError> {
        self.base.table(ResourceActions::new)
    }
}
