pub mod error;
pub mod payload;
pub mod settings;

#[path = "lago/tests/async_.rs"]
mod async_;
#[path = "lago/tests/charge_by.rs"]
mod charge_by;
#[path = "lago/tests/common_logic.rs"]
mod common_logic;
#[path = "lago/tests/init.rs"]
mod init;
#[path = "lago/tests/post.rs"]
mod post;
#[path = "lago/tests/support.rs"]
mod support;
#[path = "lago/tests/validate.rs"]
mod validate;

use url::Url;

use crate::finops::metering::event::MeteringEvent;
use crate::finops::metering::transport::{
    MeteringPost, MeteringResponse, MeteringSender, events_url,
};
use error::LagoError;
use settings::LagoSettings;

pub const CONTENT_TYPE: &str = "application/json";

pub struct LagoLogger<Sender> {
    pub settings: LagoSettings,
    pub sender: Sender,
}

impl<Sender: MeteringSender> LagoLogger<Sender> {
    pub async fn log_success_event(
        &self,
        event: &MeteringEvent<'_>,
    ) -> Result<(), LagoError<Sender::Error>> {
        self.deliver(event).await?;
        Ok(())
    }

    // LiteLLM's async path additionally logs the accepted event at debug level
    pub async fn async_log_success_event(
        &self,
        event: &MeteringEvent<'_>,
    ) -> Result<(), LagoError<Sender::Error>> {
        let response = self.deliver(event).await?;
        tracing::debug!(body = %response.body, "Logged Lago Object");
        Ok(())
    }

    async fn deliver(
        &self,
        event: &MeteringEvent<'_>,
    ) -> Result<MeteringResponse, LagoError<Sender::Error>> {
        let payload = payload::build(&self.settings, event)?;
        let body = serde_json::to_vec(&payload)?;
        let url = Url::parse(&events_url(&self.settings.api_base)).map_err(LagoError::Url)?;
        let response = self
            .sender
            .post(MeteringPost {
                url,
                bearer: &self.settings.api_key,
                content_type: CONTENT_TYPE,
                body,
            })
            .await
            .map_err(LagoError::Send)?;
        if !(200..300).contains(&response.status) {
            tracing::debug!(body = %response.body, "Error Message from Lago");
            return Err(LagoError::Status {
                status: response.status,
            });
        }
        Ok(response)
    }
}
