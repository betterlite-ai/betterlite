pub mod credentials;
pub mod event;
pub mod settings;
pub mod transport;

mod tests;

use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::loader::EnvironmentLookup;
use tokio_util::task::TaskTracker;

use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::fields::payload::StandardLoggingPayload;
use crate::s3_transport::batch_element::{ContentType, S3BatchLoggingElement};
use crate::s3_transport::transport::{S3Transport, UploadOutcome};
use settings::Settings;

pub const CALLBACK_NAME: &str = "s3";

struct Uploads {
    settings: Settings,
    transport: Option<S3Transport>,
    environment: Arc<dyn EnvironmentLookup + Send + Sync>,
}

impl Uploads {
    async fn upload(&self, payload: Option<&StandardLoggingPayload>) {
        let Some(standard) = payload else {
            return;
        };
        let Some(transport) = self.transport.as_ref() else {
            tracing::error!("s3 Layer Error - no s3_bucket_name is configured");
            return;
        };
        let object = match event::build(standard, &self.settings, self.environment.as_ref()) {
            Ok(object) => object,
            Err(error) => {
                tracing::error!("s3 Layer Error - {error}");
                return;
            }
        };
        let element = S3BatchLoggingElement {
            payload: standard,
            s3_object_key: object.object_key,
            s3_object_download_filename: object.download_filename,
            body: Some(object.body),
            content_type: ContentType::Json,
            retrying_since: None,
        };
        if transport.async_upload_data_to_s3(&element).await != UploadOutcome::Delivered {
            tracing::error!(
                key = %element.s3_object_key,
                "s3 Layer Error - the object was not delivered"
            );
        }
    }
}

pub struct S3Logger {
    uploads: Arc<Uploads>,
    tasks: TaskTracker,
}

impl S3Logger {
    #[must_use]
    pub fn new(
        settings: Settings,
        transport: Option<S3Transport>,
        environment: Arc<dyn EnvironmentLookup + Send + Sync>,
        tasks: TaskTracker,
    ) -> Self {
        Self {
            uploads: Arc::new(Uploads {
                settings,
                transport,
                environment,
            }),
            tasks,
        }
    }
}

#[async_trait]
impl CallbackLogger for S3Logger {
    fn callback_name(&self) -> &str {
        CALLBACK_NAME
    }

    // LiteLLM runs legacy S3 on an executor, so the sync hook puts on the server's tracker
    fn log_success_event(&self, event: &LoggerEvent) -> Result<(), LoggerError> {
        let uploads = Arc::clone(&self.uploads);
        let payload = Arc::clone(&event.payload);
        self.tasks.spawn(async move {
            uploads.upload(Some(payload.as_ref())).await;
        });
        Ok(())
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.uploads.upload(Some(event.payload.as_ref())).await;
        Ok(())
    }
}
