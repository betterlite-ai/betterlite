use super::ports::CallTimes;
use super::state::LoggingHandle;
use crate::call_setup::RequestMap;
use crate::call_setup::correlation::CallOutcome;

// the response and the call it answers, read from the request the deployment saw
pub struct MetadataUpdate<'update> {
    pub outcome: &'update CallOutcome,
    pub model: Option<&'update str>,
    pub request: &'update RequestMap,
    pub times: CallTimes,
}

// B004's update_response_metadata: hidden params and timing metrics on the response, or on the logging object
pub trait ResponseMetadataUpdater: Send + Sync {
    fn update(&self, logging: &LoggingHandle, update: MetadataUpdate<'_>);
}
