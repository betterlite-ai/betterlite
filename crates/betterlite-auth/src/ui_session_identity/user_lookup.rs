use async_trait::async_trait;

use crate::auth_user_lookup::error::UserLookupError;
use crate::auth_user_lookup::lookup::{UserLookup, UserLookupRequest};

use super::UiSessionTeams;

#[async_trait]
impl UiSessionTeams for UserLookup {
    fn has_database(&self) -> bool {
        self.store.is_some()
    }

    async fn user_teams(
        &self,
        user_id: &str,
        fresh: bool,
    ) -> Result<Option<Vec<String>>, UserLookupError> {
        // get_user_object's keyword arguments: no upsert, no SSO or email key, check_db_only=fresh
        let request = UserLookupRequest {
            user_id: Some(user_id),
            sso_user_id: None,
            user_email: None,
            user_id_upsert: false,
            check_db_only: Some(fresh),
        };
        let user = self.get_user_object(request).await?;
        Ok(user.map(|row| row.teams))
    }
}

mod tests;
