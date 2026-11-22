#![cfg(test)]

mod assistant_thread_run_cases;
mod container_video_vector_cases;
mod files_batches_cases;
mod fine_tuning_cases;
mod fixture;
mod passthrough_cases;
mod realtime_cases;
mod responses_search_cases;
mod unsupported_operation;

use std::future::Future;

use rstest::rstest;
use tokio::runtime::Builder;

#[derive(Clone, Copy)]
enum Family {
    FilesBatches,
    FineTuning,
    AssistantThreadRun,
    ResponsesSearch,
    Realtime,
    ContainerVideoVector,
    Passthrough,
}

fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the runtime builds")
        .block_on(future)
}

// One row per azure-resource-dispatch oracle case file, each driving the production dispatcher
#[rstest]
#[case::files_batches(Family::FilesBatches)]
#[case::fine_tuning(Family::FineTuning)]
#[case::assistant_thread_run(Family::AssistantThreadRun)]
#[case::responses_search(Family::ResponsesSearch)]
#[case::realtime(Family::Realtime)]
#[case::container_video_vector(Family::ContainerVideoVector)]
#[case::passthrough(Family::Passthrough)]
fn azure_resource_dispatch_cases(#[case] family: Family) {
    match family {
        Family::FilesBatches => files_batches_cases::check(),
        Family::FineTuning => fine_tuning_cases::check(),
        Family::AssistantThreadRun => block_on(assistant_thread_run_cases::check()),
        Family::ResponsesSearch => responses_search_cases::check(),
        Family::Realtime => realtime_cases::check(),
        Family::ContainerVideoVector => container_video_vector_cases::check(),
        Family::Passthrough => passthrough_cases::check(),
    }
}
