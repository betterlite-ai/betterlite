use super::ports::CallTimes;
use super::state::LoggingHandle;
use crate::call_setup::RequestMap;
use crate::call_setup::correlation::CallOutcome;

// B004's update_response_metadata: hidden params and timing metrics on the response, or on the logging object
pub trait ResponseMetadataUpdater: Send + Sync {
    fn update(
        &self,
        outcome: &CallOutcome,
        logging: &LoggingHandle,
        model: Option<&str>,
        request: &RequestMap,
        times: CallTimes,
    );
}
