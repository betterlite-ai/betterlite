use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;

mod assistant_thread_run;
mod container_video_vector;
mod files_batches;
mod fine_tuning;
mod passthrough;
mod realtime;
mod responses_search;
mod support;
mod tests;

pub(crate) struct Op;

// The oracle calls one operation per input; `case` names the family the way the Python adapter does
#[derive(Debug, Deserialize)]
#[serde(tag = "case", rename_all = "snake_case")]
pub(crate) enum Input {
    FilesBatches(files_batches::Input),
    FineTuning(fine_tuning::Input),
    AssistantThreadRun(assistant_thread_run::Input),
    ResponsesSearch(responses_search::Input),
    Realtime(realtime::Input),
    ContainerVideoVector(container_video_vector::Input),
    Passthrough(passthrough::Input),
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::FilesBatches(values) => files_batches::run(values),
            Input::FineTuning(values) => fine_tuning::run(values),
            Input::AssistantThreadRun(values) => assistant_thread_run::run(values),
            Input::ResponsesSearch(values) => responses_search::run(values),
            Input::Realtime(values) => realtime::run(values),
            Input::ContainerVideoVector(values) => container_video_vector::run(values),
            Input::Passthrough(values) => passthrough::run(values),
        }
    }
}
