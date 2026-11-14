use async_trait::async_trait;

use crate::smtp_send::SendEmailError;

#[async_trait]
pub trait EmailSender: Send + Sync {
    async fn send(
        &self,
        receiver: Option<&str>,
        subject: &str,
        html: &str,
    ) -> Result<(), SendEmailError>;
}
