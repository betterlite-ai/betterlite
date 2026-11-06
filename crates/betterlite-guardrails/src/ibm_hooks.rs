use crate::ibm_detector_client::client::IbmDetectorClient;

// Only the event type differs between the two input hooks, so one macro covers both.
macro_rules! input_hook {
    ($name:ident, $event:expr) => {
        pub async fn $name(
            guardrail: &IbmGuardrail,
            data: &mut PromptData,
        ) -> Result<(), IbmHookError> {
            if !should_run_guardrail(guardrail, $event) {
                return Ok(());
            }
            let contents = iter_message_text(data);
            if !contents.is_empty() {
                check_contents(guardrail, &contents).await?;
            }
            record_applied_guardrail(data, &guardrail.client.config.guardrail_name);
            Ok(())
        }
    };
}

pub mod class_methods;
pub mod config;
pub mod data;
pub mod during_call;
pub mod error;
pub mod post_call;
pub mod pre_call;
pub mod response;
pub mod streaming;

mod detection;

use config::EventHook;

pub struct IbmGuardrail {
    pub client: IbmDetectorClient,
    pub event_hooks: Option<Vec<EventHook>>,
}

#[path = "ibm_hooks/tests/support.rs"]
mod test_support;
