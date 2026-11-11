pub mod client;
pub mod projection;
pub mod settings;
pub mod signing;

mod tests;

use std::sync::Arc;

use async_trait::async_trait;

use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::fields::payload::StandardLoggingPayload;

use client::DynamoClient;
use settings::Settings;

pub const CALLBACK_NAME: &str = "dynamodb";

pub struct DynamoDbLogger<C> {
    pub settings: Settings,
    pub client: C,
}

impl<C: DynamoClient> DynamoDbLogger<C> {
    pub async fn log_event(&self, payload: Option<&StandardLoggingPayload>) {
        let Some(standard) = payload else {
            return;
        };
        let item = match projection::item(standard) {
            Ok(item) => item,
            Err(error) => {
                tracing::debug!("DynamoDB Layer Error - {error}");
                return;
            }
        };
        if let Err(error) = self.client.put_item(&self.settings.table_name, &item).await {
            tracing::debug!(
                table = %self.settings.table_name,
                region = %self.settings.region_name,
                "DynamoDB Layer Error - {error}"
            );
        }
    }
}

#[async_trait]
impl<C: DynamoClient> CallbackLogger for DynamoDbLogger<C> {
    fn callback_name(&self) -> &str {
        CALLBACK_NAME
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.log_event(Some(event.payload.as_ref())).await;
        Ok(())
    }
}
