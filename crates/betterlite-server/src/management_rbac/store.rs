use std::convert::Infallible;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_auth::auth_user_lookup::cache::UserCache;

use super::guard::AdminPrivileges;
use crate::management_admins::ports::{AdminUserReads, TeamReads};
use crate::management_admins::privileges::user_has_admin_privileges;
use crate::management_admins::scope::AdminCaller;

// the proxy's user reads, its database when one is connected and its user cache; LiteLLM passes no logger here
pub struct StorePrivileges<'reads> {
    pub users: &'reads dyn AdminUserReads,
    pub teams: Option<&'reads dyn TeamReads>,
    pub cache: Arc<dyn UserCache>,
}

#[async_trait]
impl AdminPrivileges for StorePrivileges<'_> {
    type Failure = Infallible;

    async fn has_admin_privileges(&self, caller: &AdminCaller<'_>) -> Result<bool, Infallible> {
        let cache = Some(Arc::clone(&self.cache));
        let granted = user_has_admin_privileges(caller, self.users, self.teams, cache).await;
        Ok(granted)
    }
}
