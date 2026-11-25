use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use betterlite_auth::identity_grants::loaders::TeamLoader;
use betterlite_auth::ui_session_identity::UiSessionTeams;
use betterlite_config::sections::management_rbac::Settings as FeatureSettings;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::vector_store_access::VectorStoreAccess;
use betterlite_core::api::vector_store_access::auth::ObjectPermissionLookup;
use betterlite_core::api::vector_store_registry::store::VectorStoreRegistry;

use super::caller::CallerView;
use super::error::ManagementReadError;
use crate::management_rbac::FeatureName;
use crate::management_rbac::guard::{AdminPrivileges, check_feature_access};

// everything the two management reads take: the database, the cache, the grant lookups and the feature switches
pub struct VectorManagementReadState<Database> {
    pub database: Option<Database>,
    pub registry: Option<Arc<Mutex<VectorStoreRegistry>>>,
    pub permissions: Arc<dyn ObjectPermissionLookup>,
    pub teams: Arc<dyn TeamLoader>,
    pub session_teams: Arc<dyn UiSessionTeams>,
    pub privileges: Arc<dyn AdminPrivileges<Failure = Infallible>>,
    pub settings: FeatureSettings,
    pub limits: ApiLimits,
}

impl<Database> VectorManagementReadState<Database> {
    #[must_use]
    pub fn access(
        &self,
    ) -> VectorStoreAccess<'_, dyn ObjectPermissionLookup, dyn TeamLoader, dyn UiSessionTeams> {
        VectorStoreAccess {
            permissions: self.permissions.as_ref(),
            teams: self.teams.as_ref(),
            session_teams: self.session_teams.as_ref(),
        }
    }

    // check_feature_access_for_user: B864's switch, run before any repository work on both routes
    pub async fn check_vector_feature(
        &self,
        caller: &CallerView,
    ) -> Result<(), ManagementReadError> {
        check_feature_access(
            caller.role,
            caller.user_id.as_deref(),
            FeatureName::VectorStores,
            &self.settings,
            self.privileges.as_ref(),
        )
        .await?;
        Ok(())
    }
}
