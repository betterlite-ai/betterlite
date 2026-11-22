use std::sync::Arc;

use async_trait::async_trait;
use betterlite_logging::budget_email::event::WebhookEvent;
use betterlite_server::key_event_sinks::email::{CreatedKey, LegacyEmail};
use betterlite_server::key_event_sinks::ports::KeyCreatedMailer;
use betterlite_server::key_events::effects::{KeyEmailError, KeyEventEmail};
use rust_decimal::prelude::ToPrimitive;
use secrecy::ExposeSecret;
use serde_json::json;
use tokio_util::task::TaskTracker;

use super::board::Board;

struct BoardMailer {
    board: Arc<Board>,
}

#[async_trait]
impl KeyCreatedMailer for BoardMailer {
    async fn send(&self, event: WebhookEvent) {
        let view = json!({
            "event": event.event.to_string(),
            "event_group": event.event_group.to_string(),
            "event_message": event.event_message,
            "token": event.token.as_ref().map(ExposeSecret::expose_secret),
            "spend": event.spend.to_f64(),
            "max_budget": event.max_budget.as_ref().and_then(ToPrimitive::to_f64),
            "user_id": event.user_id,
            "team_id": event.team_id,
            "key_alias": event.key_alias,
        });
        self.board.record(json!(["email", view]));
    }
}

// B898's created-email decision is kept, and its task is awaited where the hook sends it, so the
// mail it records sits at the email step of the trace rather than after the hook returned
pub(crate) struct WaitingEmail {
    pub(crate) alerting: Vec<String>,
    pub(crate) board: Arc<Board>,
    pub(crate) tasks: TaskTracker,
}

#[async_trait]
impl KeyEventEmail for WaitingEmail {
    async fn send_created(&self, response: CreatedKey) -> Result<(), KeyEmailError> {
        let email = LegacyEmail {
            alerting: &self.alerting,
            mailer: Arc::new(BoardMailer {
                board: Arc::clone(&self.board),
            }),
            tasks: &self.tasks,
        };
        let sent = email.send_key_created_email(response);
        self.tasks.close();
        self.tasks.wait().await;
        sent.map_err(KeyEmailError::from)
    }

    async fn send_rotated(
        &self,
        _response: &str,
        _existing_key_alias: Option<&str>,
    ) -> Result<(), KeyEmailError> {
        Ok(())
    }
}
