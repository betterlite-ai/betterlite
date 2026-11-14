pub mod branding;
pub mod event;
pub mod sender;
pub mod team_members;
pub mod templates;

mod tests;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::team_repository::repository::TeamQueryError;

use crate::smtp_send::SendEmailError;

use self::event::{EntityType, WebhookEvent, WebhookEventType};
use self::sender::EmailSender;
use self::templates::{TeamBudgetEmail, UserBudgetEmail};

#[derive(Debug, thiserror::Error)]
pub enum BudgetEmailError {
    #[error("not connected to the database, so the team member emails cannot be read")]
    NoDatabase,
    #[error("the team lookup failed")]
    Team {
        #[from]
        source: TeamQueryError,
    },
    #[error("the team member email query failed")]
    Database {
        #[from]
        source: DbError,
    },
    #[error(transparent)]
    Send(#[from] SendEmailError),
}

pub async fn send_team_budget_alert(
    event: &WebhookEvent,
    environment: &(dyn EnvironmentLookup + Send + Sync),
    database: Option<&PostgresStore>,
    sender: &dyn EmailSender,
) -> Result<bool, BudgetEmailError> {
    let branding = branding::resolve_branding(environment);
    let recipient_emails =
        team_members::get_all_team_member_emails(database, event.team_id.as_deref()).await?;
    let recipients = recipient_emails.join(",");
    tracing::debug!("Email Alerting: Sending team budget alert to {recipients}");
    let email = TeamBudgetEmail {
        logo_url: &branding.logo_url,
        team_alias: event.team_alias.as_deref(),
        max_budget: event.max_budget,
        spend: event.spend,
        support_contact: &branding.support_contact,
    };
    let subject = format!(
        "LiteLLM {} for Team {}",
        event.event_message,
        event.team_alias.as_deref().unwrap_or("None")
    );
    sender
        .send(
            Some(recipients.as_str()),
            &subject,
            &templates::render_team_budget_email(&email),
        )
        .await?;
    Ok(false)
}

pub async fn send_email_alert_using_smtp(
    event: &WebhookEvent,
    _alert_type: AlertType,
    environment: &(dyn EnvironmentLookup + Send + Sync),
    database: Option<&PostgresStore>,
    sender: &dyn EmailSender,
) -> Result<bool, BudgetEmailError> {
    let branding = branding::resolve_branding(environment);
    if event.user_email.is_none() {
        tracing::error!(
            "Email Alerting: Trying to send email alert to no recipient, event={} user_id={}",
            event.event,
            event.user_id.as_deref().unwrap_or("")
        );
    }
    let html = if event.event == WebhookEventType::BudgetCrossed {
        let email = UserBudgetEmail {
            logo_url: &branding.logo_url,
            user_name: event.user_id.as_deref(),
            max_budget: event.max_budget,
            support_contact: &branding.support_contact,
        };
        templates::render_user_budget_email(&email)
    } else {
        "Alert from LiteLLM Server".to_owned()
    };
    let subject = format!("LiteLLM: {}", event.event_message);
    sender
        .send(event.user_email.as_deref(), &subject, &html)
        .await?;
    if event.event_group == EntityType::Team {
        send_team_budget_alert(event, environment, database, sender).await?;
    }
    Ok(false)
}
