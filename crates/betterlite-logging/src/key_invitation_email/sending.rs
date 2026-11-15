use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_repository::repository::{TeamDatabase, TeamRepository};
use betterlite_db::user_repository::UserRepository;
use rust_decimal::prelude::ToPrimitive as _;
use secrecy::ExposeSecret as _;

use super::link::{InvitationCreator, construct_user_invitation_link};
use super::templates::{
    KeyCreatedEmail, UserInvitedEmail, render_key_created_email, render_user_invited_email,
};
use super::{KeyInvitationEmailError, KeyInvitationEmailer};
use crate::alert_dispatch::destination::AlertDestination;
use crate::budget_email::branding::{Branding, resolve_branding};
use crate::budget_email::event::{WebhookEvent, WebhookEventType};

const DEFAULT_PROXY_BASE_URL: &str = "http://0.0.0.0:4000";
const DEFAULT_TEAM_NAME: &str = "Default Team";

impl<C> KeyInvitationEmailer<'_, C>
where
    C: InvitationCreator,
{
    #[must_use]
    // ast-grep-ignore: no-long-names
    pub async fn send_key_created_or_user_invited_email(&self, event: &WebhookEvent) -> bool {
        match self.try_send(event).await {
            Ok(sent) => sent,
            Err(error) => {
                tracing::error!("Error sending email alert: {error}");
                false
            }
        }
    }

    async fn try_send(&self, event: &WebhookEvent) -> Result<bool, KeyInvitationEmailError> {
        if !self
            .alerting
            .is_some_and(|alerting| alerting.contains(&AlertDestination::Email))
        {
            tracing::error!(
                "Error sending email alert - 'email' not in the alerting list: {:?}",
                self.alerting
            );
            return Ok(false);
        }
        let branding = resolve_branding(self.environment);
        let base_url = self
            .environment
            .lookup("PROXY_BASE_URL")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_PROXY_BASE_URL.to_owned());
        let recipient_email = self.recipient_email(event).await?;
        if recipient_email.is_none() {
            tracing::error!(
                "Trying to send email alert to no recipient, event={} user_id={}",
                event.event,
                event.user_id.as_deref().unwrap_or("")
            );
        }
        let html = self
            .email_body(event, &branding, &base_url, recipient_email.as_deref())
            .await?;
        let subject = format!("LiteLLM: {}", event.event_message);
        self.sender
            .send(recipient_email.as_deref(), &subject, &html)
            .await?;
        Ok(true)
    }

    async fn recipient_email(
        &self,
        event: &WebhookEvent,
    ) -> Result<Option<String>, KeyInvitationEmailError> {
        let (None, Some(user_id), Some(database)) = (
            event.user_email.as_deref(),
            event.user_id.as_deref(),
            self.database,
        ) else {
            return Ok(event.user_email.clone());
        };
        let client = database.session().await?;
        let users = UserRepository {
            db: Some(&client),
            writer_db: None,
            use_writer: false,
        };
        let found = users.find_by_id(user_id).await?;
        Ok(found.and_then(|user| user.user_email))
    }

    async fn email_body(
        &self,
        event: &WebhookEvent,
        branding: &Branding,
        base_url: &str,
        recipient_email: Option<&str>,
    ) -> Result<String, KeyInvitationEmailError> {
        if event.event == WebhookEventType::KeyCreated {
            let email = KeyCreatedEmail {
                email_logo_url: &branding.logo_url,
                recipient_email,
                key_budget: event.max_budget.and_then(|budget| budget.to_f64()),
                key_token: event
                    .token
                    .as_ref()
                    .map(|token| token.expose_secret().as_str()),
                base_url,
                email_support_contact: &branding.support_contact,
            };
            return Ok(render_key_created_email(&email));
        }
        if event.event == WebhookEventType::InternalUserCreated {
            let team_name = self.team_name(event.team_id.as_deref()).await?;
            let invitation_link = construct_user_invitation_link(
                event.user_id.as_deref(),
                base_url,
                self.database,
                self.invitation_creator,
            )
            .await;
            let email = UserInvitedEmail {
                email_logo_url: &branding.logo_url,
                recipient_email,
                team_name: &team_name,
                base_url: &invitation_link,
                email_support_contact: &branding.support_contact,
            };
            return Ok(render_user_invited_email(&email));
        }
        tracing::error!(
            "Trying to send email alert on unknown webhook event, event={}",
            event.event
        );
        Ok("Alert from LiteLLM Server".to_owned())
    }

    async fn team_name(&self, team_id: Option<&str>) -> Result<String, KeyInvitationEmailError> {
        let (Some(team_id), Some(database)) = (team_id, self.database) else {
            return Ok(DEFAULT_TEAM_NAME.to_owned());
        };
        let teams = TeamRepository {
            database: TeamDatabase {
                db: database,
                writer_db: database,
            },
            use_writer: false,
        };
        let Some(record) = teams.table().find_unique(team_id).await? else {
            return Ok(DEFAULT_TEAM_NAME.to_owned());
        };
        let Some(JsonData::String(alias)) = record.fields.get("team_alias") else {
            return Ok("-".to_owned());
        };
        if alias.is_empty() {
            return Ok("-".to_owned());
        }
        Ok(alias.clone())
    }
}
