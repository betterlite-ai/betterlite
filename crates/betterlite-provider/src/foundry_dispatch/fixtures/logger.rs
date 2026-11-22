use crate::foundry_dispatch::post_call::FoundryLogger;
use crate::openai_dispatch::logging::LoggedCall;

// Records every post_call the composition makes, so a case can compare the count and the fields
#[derive(Default)]
pub struct RecordingLogger {
    pub calls: Vec<LoggedCall>,
}

impl FoundryLogger for RecordingLogger {
    fn post_call(&mut self, record: LoggedCall) {
        self.calls.push(record);
    }
}
