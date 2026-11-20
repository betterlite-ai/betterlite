#![cfg(test)]

mod fixture;
mod image_edit_cases;
mod image_generation_cases;
mod messages_count_cases;
mod passthrough_cases;
mod rerank_cases;
mod responses_decisions_cases;
mod unsupported_operation;
mod vector_search_cases;

use std::future::Future;

use rstest::rstest;
use tokio::runtime::Builder;

#[derive(Clone, Copy)]
enum Family {
    ImageEdit,
    Rerank,
    ResponsesDecisions,
    VectorSearch,
    Passthrough,
    ImageGeneration,
    MessagesCount,
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

// One row per foundry-resource-dispatch oracle case file, each driving the production dispatcher
#[rstest]
#[case::image_edit(Family::ImageEdit)]
#[case::rerank(Family::Rerank)]
#[case::responses_decisions(Family::ResponsesDecisions)]
#[case::vector_search(Family::VectorSearch)]
#[case::passthrough(Family::Passthrough)]
#[case::image_generation(Family::ImageGeneration)]
#[case::messages_count(Family::MessagesCount)]
fn foundry_resource_dispatch_cases(#[case] family: Family) {
    match family {
        Family::ImageEdit => block_on(image_edit_cases::check()),
        Family::Rerank => block_on(rerank_cases::check()),
        Family::ResponsesDecisions => block_on(responses_decisions_cases::check()),
        Family::VectorSearch => block_on(vector_search_cases::check()),
        Family::Passthrough => block_on(passthrough_cases::check()),
        Family::ImageGeneration => block_on(image_generation_cases::check()),
        Family::MessagesCount => messages_count_cases::check(),
    }
}
