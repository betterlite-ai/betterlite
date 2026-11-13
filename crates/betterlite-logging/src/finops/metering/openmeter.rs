pub mod error;
pub mod payload;
pub mod settings;

#[path = "openmeter/tests/async_.rs"]
mod async_;
#[path = "openmeter/tests/common_logic.rs"]
mod common_logic;
#[path = "openmeter/tests/init.rs"]
mod init;
#[path = "openmeter/tests/post.rs"]
mod post;
#[path = "openmeter/tests/subject.rs"]
mod subject;
#[path = "openmeter/tests/support.rs"]
mod support;
#[path = "openmeter/tests/validate.rs"]
mod validate;

use url::Url;

use crate::finops::metering::clock::UtcClock;
use crate::finops::metering::event::MeteringEvent;
use crate::finops::metering::transport::{MeteringPost, MeteringSender, events_url};
use error::OpenMeterError;
use settings::OpenMeterSettings;

pub const CONTENT_TYPE: &str = "application/cloudevents+json";

pub struct OpenMeterLogger<Sender, Time> {
    pub settings: OpenMeterSettings,
    pub sender: Sender,
    pub clock: Time,
}

impl<Sender: MeteringSender, Time: UtcClock> OpenMeterLogger<Sender, Time> {
    pub async fn log_success_event(
        &self,
        event: &MeteringEvent<'_>,
    ) -> Result<(), OpenMeterError<Sender::Error>> {
        self.deliver(event).await?;
        Ok(())
    }

    pub async fn async_log_success_event(
        &self,
        event: &MeteringEvent<'_>,
    ) -> Result<(), OpenMeterError<Sender::Error>> {
        self.deliver(event).await?;
        Ok(())
    }

    async fn deliver(
        &self,
        event: &MeteringEvent<'_>,
    ) -> Result<(), OpenMeterError<Sender::Error>> {
        let payload = payload::build(&self.settings, &self.clock, event)?;
        let body = serde_json::to_vec(&payload)?;
        let url = Url::parse(&events_url(&self.settings.endpoint)).map_err(OpenMeterError::Url)?;
        let response = self
            .sender
            .post(MeteringPost {
                url,
                bearer: &self.settings.api_key,
                content_type: CONTENT_TYPE,
                body,
            })
            .await
            .map_err(OpenMeterError::Send)?;
        if !(200..300).contains(&response.status) {
            return Err(OpenMeterError::Status {
                status: response.status,
                body: response.body,
            });
        }
        Ok(())
    }
}
