use async_trait::async_trait;
use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_auth::auth_team_lookup::lookup::{TeamLookup, TeamRequest, TeamSource};
use betterlite_auth::auth_team_lookup::team::get_team_object;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_logging::budget_email::event::WebhookEvent;

// get_team_object as the secret params read it; Ok(None) is a proxy without a database, where nothing is looked up
#[async_trait]
pub trait TeamDirectory: Send + Sync {
    async fn team(
        &self,
        team_id: &str,
    ) -> Result<Option<LiteLLMTeamTableCachedObj>, TeamLookupError>;
}

#[async_trait]
impl TeamDirectory for TeamLookup {
    async fn team(
        &self,
        team_id: &str,
    ) -> Result<Option<LiteLLMTeamTableCachedObj>, TeamLookupError> {
        if self.database.is_none() {
            return Ok(None);
        }
        let request = TeamRequest {
            team_id,
            source: TeamSource::CacheThenDatabase,
            team_id_upsert: false,
        };
        get_team_object(self, request).await.map(Some)
    }
}

// SlackAlerting.send_key_created_or_user_invited_email, which B4023's emailer implements
#[async_trait]
pub trait KeyCreatedMailer: Send + Sync {
    async fn send(&self, event: WebhookEvent);
}
